use std::{collections::HashMap, env, fs::{self, OpenOptions}, io::Write, net::SocketAddr, path::PathBuf, sync::Arc, time::SystemTime};
use anyhow::{Context, Result};
use axum::{extract::State, http::{HeaderMap, StatusCode}, response::IntoResponse, routing::{get, post}, Json, Router};
use clap::{Parser, Subcommand};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "real-api-router", about = "Policy-driven router over real-api-pricing snapshots")]
struct Cli { #[arg(long, default_value = "router.toml")] config: PathBuf, #[command(subcommand)] command: Command }
#[derive(Subcommand)] enum Command { Serve, Validate, Route { #[arg(long, default_value = "balanced")] mode: String } }

#[derive(Debug, Clone, Deserialize)]
struct Config { bind: Option<String>, data_file: String, audit_file: Option<String>, max_snapshot_age_hours: Option<u64>, providers: Vec<ProviderConfig> }
#[derive(Debug, Clone, Deserialize)]
struct ProviderConfig { id: String, kind: String, base_url: String, api_key_env: Option<String>, regions: Option<Vec<String>> }
#[derive(Debug, Clone, Serialize)]
struct Candidate { provider: String, model: String, region: String, source_kind: String, effective_cost: f64, capability: f64, confidence: String, raw: Value }
#[derive(Debug, Clone, Serialize, Default)]
struct ProviderState { failures: u32, cooldown_until_unix: u64, latency_ms_ewma: f64 }
#[derive(Clone)] struct AppState { cfg: Arc<Config>, catalog: Arc<Vec<Candidate>>, snapshot_age_hours: u64, states: Arc<RwLock<HashMap<String, ProviderState>>>, http: Client }

#[derive(Debug, Deserialize)]
struct RouteRequest { mode: Option<String>, maximum_cost: Option<f64>, minimum_capability: Option<f64>, allowed_providers: Option<Vec<String>>, blocked_providers: Option<Vec<String>>, allowed_regions: Option<Vec<String>>, local_only: Option<bool>, require_approval: Option<bool>, approved: Option<bool> }
#[derive(Debug, Clone, Serialize)]
struct Ranked { candidate: Candidate, score: f64, reasons: Vec<String>, provider_state: ProviderState }
#[derive(Debug, Serialize)]
struct RouteResponse { request_id: Uuid, status: String, snapshot_age_hours: u64, selected: Option<Ranked>, alternatives: Vec<Ranked>, note: String }
#[derive(Debug, Deserialize, Serialize, Clone)]
struct ChatMessage { role: String, content: Value }
#[derive(Debug, Deserialize)]
struct ChatRequest { #[serde(flatten)] route: RouteRequest, messages: Vec<ChatMessage>, max_tokens: Option<u32>, temperature: Option<f64> }

fn text(v: &Value, names: &[&str]) -> String { names.iter().find_map(|n| v.get(*n).and_then(Value::as_str)).unwrap_or_default().to_string() }
fn number(v: &Value, names: &[&str]) -> f64 { names.iter().find_map(|n| v.get(*n).and_then(Value::as_f64).or_else(|| v.get(*n).and_then(Value::as_str).and_then(|s| s.parse().ok()))).unwrap_or(f64::INFINITY) }
fn finite(v: f64, fallback: f64) -> f64 { if v.is_finite() { v } else { fallback } }

fn load_catalog(path: &str) -> Result<Vec<Candidate>> {
 let bytes = fs::read(path).with_context(|| format!("read pricing snapshot {path}"))?;
 let root: Value = serde_json::from_slice(&bytes).context("points.json must be valid JSON")?;
 let rows = root.as_array().cloned().or_else(|| root.get("points").and_then(Value::as_array).cloned()).or_else(|| root.get("data").and_then(Value::as_array).cloned()).context("expected a JSON array or points/data array")?;
 let mut out = Vec::new();
 for raw in rows {
   let provider = text(&raw, &["provider", "vendor"]); let model = text(&raw, &["model", "model_id", "served_model", "name"]);
   let cost = number(&raw, &["real_unit_price", "effective_cost", "unit_price", "price", "price_per_million"]);
   if provider.is_empty() || model.is_empty() || !cost.is_finite() { continue; }
   out.push(Candidate { provider, model, region: { let r=text(&raw,&["region"]); if r.is_empty(){"GLOBAL".into()}else{r} }, source_kind: { let k=text(&raw,&["kind","plan_type","source_type"]); if k.is_empty(){"unknown".into()}else{k} }, effective_cost: cost, capability: finite(number(&raw,&["score","capability","benchmark_score"]),0.0), confidence: {let c=text(&raw,&["confidence"]);if c.is_empty(){"unknown".into()}else{c}}, raw });
 }
 if out.is_empty() { anyhow::bail!("no routable rows found: inspect field mappings in load_catalog") }
 Ok(out)
}

fn snapshot_age(path: &str) -> Result<u64> { Ok(SystemTime::now().duration_since(fs::metadata(path)?.modified()?)?.as_secs()/3600) }
fn provider_for<'a>(cfg: &'a Config, name: &str) -> Option<&'a ProviderConfig> { cfg.providers.iter().find(|p| p.id.eq_ignore_ascii_case(name)) }
fn is_local(p: &ProviderConfig) -> bool { p.regions.as_ref().is_some_and(|r| r.iter().any(|x| x.eq_ignore_ascii_case("LOCAL"))) || p.base_url.contains("127.0.0.1") || p.base_url.contains("localhost") }

async fn rank(state: &AppState, req: &RouteRequest) -> Vec<Ranked> {
 let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(); let states = state.states.read().await; let mode = req.mode.as_deref().unwrap_or("balanced"); let mut ranked=Vec::new();
 for c in state.catalog.iter() {
   let Some(p)=provider_for(&state.cfg,&c.provider) else { continue };
   // Plans/subscriptions are never transport targets. Only configured official/local providers are callable.
   if c.source_kind.to_lowercase().contains("subscription") { continue; }
   if req.local_only.unwrap_or(false) && !is_local(p) { continue }
   if req.maximum_cost.is_some_and(|x| c.effective_cost>x) || req.minimum_capability.is_some_and(|x| c.capability<x) { continue }
   if req.allowed_providers.as_ref().is_some_and(|v| !v.iter().any(|x| x.eq_ignore_ascii_case(&c.provider))) { continue }
   if req.blocked_providers.as_ref().is_some_and(|v| v.iter().any(|x| x.eq_ignore_ascii_case(&c.provider))) { continue }
   if req.allowed_regions.as_ref().is_some_and(|v| !v.iter().any(|x| x.eq_ignore_ascii_case(&c.region) || x.eq_ignore_ascii_case("GLOBAL"))) { continue }
   let ps=states.get(&c.provider).cloned().unwrap_or_default(); if ps.cooldown_until_unix>now { continue }
   let mut score=match mode { "cheapest" => c.effective_cost, "best" => -c.capability, "coding" => c.effective_cost/(1.0+c.capability), "private"|"local_only" => if is_local(p){c.effective_cost-10000.0}else{c.effective_cost+10000.0}, _ => c.effective_cost/(1.0+c.capability) };
   score += ps.latency_ms_ewma/100000.0 + ps.failures as f64*100.0;
   ranked.push(Ranked { candidate:c.clone(), score, reasons:vec![format!("mode={mode}"), format!("effective_cost={}",c.effective_cost),format!("capability={}",c.capability),format!("latency_ewma_ms={}",ps.latency_ms_ewma)], provider_state:ps });
 } ranked.sort_by(|a,b| a.score.total_cmp(&b.score)); ranked
}

async fn route(State(state): State<AppState>, Json(req): Json<RouteRequest>) -> impl IntoResponse {
 let ranked=rank(&state,&req).await; let selected=ranked.first().cloned(); let gated=req.require_approval.unwrap_or(false) && !req.approved.unwrap_or(false);
 let response=RouteResponse { request_id:Uuid::new_v4(), status:if gated{"approval_required".into()}else if selected.is_some(){"routed".into()}else{"no_eligible_route".into()}, snapshot_age_hours:state.snapshot_age_hours, selected, alternatives:ranked.into_iter().skip(1).take(5).collect(), note:if gated{"Proposal only: resubmit with approved=true before a paid request.".into()}else{"Selection uses the checked-in pricing snapshot plus local health state; it is not a live provider-price guarantee.".into()} }; Json(response)
}

async fn catalog(State(state): State<AppState>) -> impl IntoResponse { Json(json!({"snapshot_age_hours":state.snapshot_age_hours,"candidates":state.catalog,"configured_providers":state.cfg.providers.iter().map(|p| &p.id).collect::<Vec<_>>() })) }
async fn health(State(state): State<AppState>) -> impl IntoResponse { let s=state.states.read().await.clone(); Json(json!({"ok":true,"snapshot_age_hours":state.snapshot_age_hours,"provider_state":s})) }

fn authorized(headers:&HeaderMap)->bool { match env::var("ROUTER_BEARER_TOKEN") { Ok(token) if !token.is_empty()=>headers.get("authorization").and_then(|v|v.to_str().ok()).is_some_and(|v|v==format!("Bearer {token}")), _=>true } }
async fn chat(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<ChatRequest>) -> impl IntoResponse {
 if !authorized(&headers) { return (StatusCode::UNAUTHORIZED,Json(json!({"error":"router bearer token required"}))).into_response() }
 let ranked=rank(&state,&body.route).await; let Some(selected)=ranked.first() else { return (StatusCode::UNPROCESSABLE_ENTITY,Json(json!({"error":"no eligible configured API route"}))).into_response() };
 if body.route.require_approval.unwrap_or(false)&&!body.route.approved.unwrap_or(false) { return (StatusCode::PRECONDITION_REQUIRED,Json(json!({"error":"approval required","proposal":selected}))).into_response() }
 let Some(p)=provider_for(&state.cfg,&selected.candidate.provider) else { return (StatusCode::BAD_GATEWAY,Json(json!({"error":"provider not configured"}))).into_response() };
 let started=std::time::Instant::now(); let result=invoke(&state.http,p,&selected.candidate.model,&body.messages,body.max_tokens,body.temperature).await; let elapsed=started.elapsed().as_millis() as f64; let mut states=state.states.write().await; let entry=states.entry(p.id.clone()).or_default();
 match result { Ok(v)=>{entry.failures=0;entry.latency_ms_ewma=if entry.latency_ms_ewma==0.0{elapsed}else{entry.latency_ms_ewma*0.8+elapsed*0.2}; audit(&state,&json!({"event":"success","provider":p.id,"model":selected.candidate.model,"latency_ms":elapsed})); (StatusCode::OK,Json(json!({"route":selected,"response":v}))).into_response()}, Err(e)=>{entry.failures+=1;entry.cooldown_until_unix=SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs()+60*entry.failures.min(10) as u64; warn!(provider=%p.id,error=%e,"provider call failed"); audit(&state,&json!({"event":"failure","provider":p.id,"error":e.to_string()})); (StatusCode::BAD_GATEWAY,Json(json!({"error":"provider call failed","provider":p.id,"detail":e.to_string()}))).into_response()} }
}

async fn invoke(http:&Client,p:&ProviderConfig,model:&str,messages:&[ChatMessage],max_tokens:Option<u32>,temperature:Option<f64>)->Result<Value>{
 let key=p.api_key_env.as_ref().map(|e|env::var(e).with_context(||format!("missing {e}"))).transpose()?;
 if p.kind=="anthropic" { let url=format!("{}/v1/messages",p.base_url.trim_end_matches('/')); let payload=json!({"model":model,"max_tokens":max_tokens.unwrap_or(1024),"temperature":temperature,"messages":messages}); let mut r=http.post(url).header("anthropic-version","2023-06-01").json(&payload);if let Some(k)=key{r=r.header("x-api-key",k)};let res=r.send().await?;let status=res.status();let body=res.json::<Value>().await?;if !status.is_success(){anyhow::bail!("HTTP {status}: {body}")}return Ok(body) }
 let url=format!("{}/chat/completions",p.base_url.trim_end_matches('/'));let payload=json!({"model":model,"messages":messages,"max_tokens":max_tokens,"temperature":temperature});let mut r=http.post(url).json(&payload);if let Some(k)=key{r=r.bearer_auth(k)};let res=r.send().await?;let status=res.status();let body=res.json::<Value>().await?;if !status.is_success(){anyhow::bail!("HTTP {status}: {body}")}Ok(body)
}
fn audit(state:&AppState,value:&Value){if let Some(path)=&state.cfg.audit_file{if let Ok(mut f)=OpenOptions::new().create(true).append(true).open(path){let _=writeln!(f,"{}",json!({"at_unix":SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),"data":value}));}}}

#[tokio::main] async fn main()->Result<()> { dotenvy::dotenv().ok(); tracing_subscriber::fmt().with_env_filter("info").init(); let cli=Cli::parse(); let cfg:Config=toml::from_str(&fs::read_to_string(&cli.config)?).context("parse router TOML")?; let catalog=load_catalog(&cfg.data_file)?;let age=snapshot_age(&cfg.data_file)?; if age>cfg.max_snapshot_age_hours.unwrap_or(168){warn!(snapshot_age_hours=age,"pricing snapshot is stale")} match cli.command { Command::Validate=>{println!("valid: {} candidates; snapshot age {}h",catalog.len(),age)}, Command::Route{mode}=>{let state=AppState{cfg:Arc::new(cfg),catalog:Arc::new(catalog),snapshot_age_hours:age,states:Arc::new(RwLock::new(HashMap::new())),http:Client::new()};println!("{}",serde_json::to_string_pretty(&rank(&state,&RouteRequest{mode:Some(mode),maximum_cost:None,minimum_capability:None,allowed_providers:None,blocked_providers:None,allowed_regions:None,local_only:None,require_approval:None,approved:None}).await)?);}, Command::Serve=>{let bind=cfg.bind.clone().unwrap_or_else(||"127.0.0.1:8787".into());let state=AppState{cfg:Arc::new(cfg),catalog:Arc::new(catalog),snapshot_age_hours:age,states:Arc::new(RwLock::new(HashMap::new())),http:Client::new()};let app=Router::new().route("/v1/health",get(health)).route("/v1/catalog",get(catalog)).route("/v1/route",post(route)).route("/v1/chat/completions",post(chat)).with_state(state);let addr:SocketAddr=bind.parse()?;info!(%addr,"router listening");axum::serve(tokio::net::TcpListener::bind(addr).await?,app).await?;} } Ok(()) }
