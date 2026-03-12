// ─────────────────────────────────────────────────────────────
//  rust/src/api/zeroclaw.rs
//
//  Flutter ↔ ZeroClaw bridge with:
//    • On-device SQLite vector memory
//    • Multi-provider LLM routing (fallback / round-robin /
//      priority)
//
//  Every public function here is exposed to Dart via
//  `flutter_rust_bridge_codegen generate`.
// ─────────────────────────────────────────────────────────────

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use tokio::sync::Mutex;

use zeroclaw::config::schema::MemoryConfig;
use zeroclaw::memory::{
    create_memory, Memory, MemoryCategory, MemoryEntry,
};

// ═════════════════════════════════════════════════════════════
//  FFI-SAFE DATA-TRANSFER OBJECTS
// ═════════════════════════════════════════════════════════════

/// Mirror of `MemoryEntry` — plain types for clean Dart codegen.
#[derive(Debug, Clone)]
pub struct MemoryEntryDto {
    pub id: String,
    pub key: String,
    pub content: String,
    pub category: String,
    pub timestamp: String,
    pub session_id: Option<String>,
    pub score: Option<f64>,
}

impl From<MemoryEntry> for MemoryEntryDto {
    fn from(e: MemoryEntry) -> Self {
        Self {
            id: e.id,
            key: e.key,
            content: e.content,
            category: e.category.to_string(),
            timestamp: e.timestamp,
            session_id: e.session_id,
            score: e.score,
        }
    }
}

/// Describes one LLM provider that can be registered.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub name: String,
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub priority: u32,
}

/// The routing strategy that decides which provider handles a request.
#[derive(Debug, Clone)]
pub enum RoutingStrategy {
    Fallback,
    RoundRobin,
    Priority,
}

/// Describes a registered provider's status.
#[derive(Debug, Clone)]
pub struct ProviderStatus {
    pub name: String,
    pub model: String,
    pub base_url: String,
    pub priority: u32,
    pub is_healthy: bool,
}

/// Overall agent status snapshot.
#[derive(Debug, Clone)]
pub struct AgentStatus {
    pub initialized: bool,
    pub memory_backend: String,
    pub data_dir: String,
    pub memory_count: u64,
    pub provider_count: u32,
    pub routing_strategy: String,
}

// ═════════════════════════════════════════════════════════════
//  INTERNAL STATE
// ═════════════════════════════════════════════════════════════

struct RegisteredProvider {
    config: ProviderConfig,
    healthy: bool,
}

struct AgentState {
    memory: Box<dyn Memory>,
    data_dir: PathBuf,
    providers: Vec<RegisteredProvider>,
    strategy: RoutingStrategy,
    round_robin_idx: AtomicUsize,
}

static STATE: OnceLock<Mutex<AgentState>> = OnceLock::new();

async fn with_state<F, T>(f: F) -> Result<T, String>
where
    F: FnOnce(&mut AgentState) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<T, String>> + Send + '_>,
    >,
{
    let mutex = STATE
        .get()
        .ok_or_else(|| "Agent not initialised. Call init_agent() first.".to_string())?;
    let mut state = mutex.lock().await;
    f(&mut state).await
}

// ─── Category helpers ───────────────────────────────────────

fn parse_category(cat: &str) -> MemoryCategory {
    match cat.trim().to_ascii_lowercase().as_str() {
        "core" => MemoryCategory::Core,
        "daily" => MemoryCategory::Daily,
        "conversation" => MemoryCategory::Conversation,
        other => MemoryCategory::Custom(other.to_string()),
    }
}

fn parse_category_filter(cat: &Option<String>) -> Option<MemoryCategory> {
    cat.as_deref().map(parse_category)
}

fn strategy_name(s: &RoutingStrategy) -> &'static str {
    match s {
        RoutingStrategy::Fallback => "fallback",
        RoutingStrategy::RoundRobin => "round_robin",
        RoutingStrategy::Priority => "priority",
    }
}

// ═════════════════════════════════════════════════════════════
//  PROVIDER ROUTING ENGINE
// ═════════════════════════════════════════════════════════════

fn pick_providers(state: &AgentState) -> Vec<usize> {
    if state.providers.is_empty() {
        return vec![];
    }

    match state.strategy {
        RoutingStrategy::Fallback => {
            let mut indices: Vec<usize> = (0..state.providers.len()).collect();
            indices.sort_by_key(|&i| state.providers[i].config.priority);
            indices
        }
        RoutingStrategy::RoundRobin => {
            let n = state.providers.len();
            let start = state.round_robin_idx.fetch_add(1, Ordering::Relaxed) % n;
            (0..n).map(|i| (start + i) % n).collect()
        }
        RoutingStrategy::Priority => {
            let best = (0..state.providers.len())
                .min_by_key(|&i| state.providers[i].config.priority)
                .unwrap_or(0);
            vec![best]
        }
    }
}

async fn call_provider(
    provider: &ProviderConfig,
    augmented_prompt: &str,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/chat/completions", provider.base_url.trim_end_matches('/'));
    
    let body = serde_json::json!({
        "model": provider.model,
        "messages": [
            {
                "role": "user",
                "content": augmented_prompt
            }
        ]
    });

    let req = client.post(&url)
        .header("Authorization", format!("Bearer {}", provider.api_key))
        .header("Content-Type", "application/json")
        .json(&body);

    let res = req.send().await.map_err(|e| format!("HTTP request failed: {e}"))?;
    
    if !res.status().is_success() {
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        return Err(format!("API Error {}: {}", status, text));
    }

    let json: serde_json::Value = res.json().await.map_err(|e| format!("Failed to parse JSON: {e}"))?;
    
    if let Some(content) = json["choices"][0]["message"]["content"].as_str() {
        Ok(content.to_string())
    } else {
        Err("Failed to extract content from response".to_string())
    }
}

async fn route_prompt(
    state: &mut AgentState,
    augmented_prompt: &str,
) -> Result<String, String> {
    let indices = pick_providers(state);

    if indices.is_empty() {
        return Err("No providers registered. Call add_provider() first.".to_string());
    }

    let mut last_error = String::new();

    for &idx in &indices {
        let config = state.providers[idx].config.clone();
        match call_provider(&config, augmented_prompt).await {
            Ok(response) => {
                state.providers[idx].healthy = true;
                return Ok(response);
            }
            Err(e) => {
                state.providers[idx].healthy = false;
                last_error = format!("[{}] {}", config.name, e);
                if !matches!(state.strategy, RoutingStrategy::Fallback) {
                    return Err(last_error);
                }
            }
        }
    }

    Err(format!(
        "All {} providers failed. Last error: {last_error}",
        indices.len()
    ))
}

// ═════════════════════════════════════════════════════════════
//  PUBLIC API  (every `pub` fn → Dart)
// ═════════════════════════════════════════════════════════════

// ─── 1. Initialise ──────────────────────────────────────────

pub async fn init_agent(
    data_dir: String,
    strategy: String,
) -> Result<AgentStatus, String> {
    if STATE.get().is_some() {
        return Err("Agent is already initialised.".to_string());
    }

    let workspace = PathBuf::from(&data_dir).join("zeroclaw");
    std::fs::create_dir_all(&workspace)
        .map_err(|e| format!("Failed to create workspace dir: {e}"))?;

    let mem_config = MemoryConfig {
        backend: "sqlite".to_string(),
        auto_save: true,
        embedding_provider: "none".to_string(), // Keep simple for mobile unless API key is set
        vector_weight: 0.7,
        keyword_weight: 0.3,
        ..MemoryConfig::default()
    };

    let memory = create_memory(&mem_config, &workspace, None)
        .map_err(|e| format!("Memory init failed: {e}"))?;

    let count = memory
        .count()
        .await
        .map_err(|e| format!("Memory count failed: {e}"))?;

    let routing = match strategy.trim().to_ascii_lowercase().as_str() {
        "round_robin" | "roundrobin" => RoutingStrategy::RoundRobin,
        "priority" => RoutingStrategy::Priority,
        _ => RoutingStrategy::Fallback,
    };

    let status = AgentStatus {
        initialized: true,
        memory_backend: memory.name().to_string(),
        data_dir: workspace.display().to_string(),
        memory_count: count as u64,
        provider_count: 0,
        routing_strategy: strategy_name(&routing).to_string(),
    };

    let state = AgentState {
        memory,
        data_dir: workspace,
        providers: Vec::new(),
        strategy: routing,
        round_robin_idx: AtomicUsize::new(0),
    };

    STATE
        .set(Mutex::new(state))
        .map_err(|_| "Race condition during init.".to_string())?;

    Ok(status)
}

// ─── 2. Provider Management ────────────────────────────────

pub async fn add_provider(
    name: String,
    api_key: String,
    model: String,
    base_url: String,
    priority: u32,
) -> Result<u32, String> {
    with_state(|state| {
        let name = name.clone();
        let api_key = api_key.clone();
        let model = model.clone();
        let base_url = base_url.clone();
        Box::pin(async move {
            state.providers.push(RegisteredProvider {
                config: ProviderConfig {
                    name,
                    api_key,
                    model,
                    base_url,
                    priority,
                },
                healthy: true,
            });
            Ok(state.providers.len() as u32)
        })
    })
    .await
}

pub async fn remove_provider(name: String) -> Result<bool, String> {
    with_state(|state| {
        let name = name.clone();
        Box::pin(async move {
            let before = state.providers.len();
            state.providers.retain(|p| p.config.name != name);
            Ok(state.providers.len() < before)
        })
    })
    .await
}

pub async fn list_providers() -> Result<Vec<ProviderStatus>, String> {
    with_state(|state| {
        Box::pin(async move {
            Ok(state
                .providers
                .iter()
                .map(|p| ProviderStatus {
                    name: p.config.name.clone(),
                    model: p.config.model.clone(),
                    base_url: p.config.base_url.clone(),
                    priority: p.config.priority,
                    is_healthy: p.healthy,
                })
                .collect())
        })
    })
    .await
}

pub async fn set_routing_strategy(strategy: String) -> Result<String, String> {
    with_state(|state| {
        let strategy = strategy.clone();
        Box::pin(async move {
            let routing = match strategy.trim().to_ascii_lowercase().as_str() {
                "round_robin" | "roundrobin" => RoutingStrategy::RoundRobin,
                "priority" => RoutingStrategy::Priority,
                "fallback" => RoutingStrategy::Fallback,
                other => return Err(format!(
                    "Unknown strategy '{other}'. Use: fallback, round_robin, priority"
                )),
            };
            let name = strategy_name(&routing).to_string();
            state.strategy = routing;
            state.round_robin_idx.store(0, Ordering::Relaxed);
            Ok(name)
        })
    })
    .await
}

pub async fn update_provider_key(
    name: String,
    new_api_key: String,
) -> Result<bool, String> {
    with_state(|state| {
        let name = name.clone();
        let new_api_key = new_api_key.clone();
        Box::pin(async move {
            if let Some(p) = state.providers.iter_mut().find(|p| p.config.name == name) {
                p.config.api_key = new_api_key;
                p.healthy = true;
                Ok(true)
            } else {
                Ok(false)
            }
        })
    })
    .await
}

pub async fn update_provider_model(
    name: String,
    new_model: String,
) -> Result<bool, String> {
    with_state(|state| {
        let name = name.clone();
        let new_model = new_model.clone();
        Box::pin(async move {
            if let Some(p) = state.providers.iter_mut().find(|p| p.config.name == name) {
                p.config.model = new_model;
                Ok(true)
            } else {
                Ok(false)
            }
        })
    })
    .await
}

// ─── 3. Chat (auto-recall + routing + auto-store) ──────────

pub async fn run_agent(
    prompt: String,
    session_id: Option<String>,
) -> Result<String, String> {
    with_state(|state| {
        let prompt = prompt.clone();
        let session_id = session_id.clone();
        Box::pin(async move {
            let sid = session_id.as_deref();

            let recalled = state
                .memory
                .recall(&prompt, 5, sid)
                .await
                .unwrap_or_default();

            let context_block = if recalled.is_empty() {
                String::new()
            } else {
                let mut ctx = String::from("\n--- Recalled Memories ---\n");
                for (i, entry) in recalled.iter().enumerate() {
                    ctx.push_str(&format!(
                        "{}. [{}] {}: {}\n",
                        i + 1, entry.category, entry.key, entry.content
                    ));
                }
                ctx.push_str("--- End ---\n\n");
                ctx
            };

            let augmented = format!("{context_block}User: {prompt}");
            let response_text = route_prompt(state, &augmented).await?;

            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let turn_key = format!("turn_{ts}");

            let _ = state.memory.store(
                &format!("{turn_key}_user"),
                &prompt,
                MemoryCategory::Conversation,
                sid,
            ).await;

            let _ = state.memory.store(
                &format!("{turn_key}_assistant"),
                &response_text,
                MemoryCategory::Conversation,
                sid,
            ).await;

            Ok(response_text)
        })
    })
    .await
}

pub async fn run_agent_with_provider(
    prompt: String,
    provider_name: String,
    session_id: Option<String>,
) -> Result<String, String> {
    with_state(|state| {
        let prompt = prompt.clone();
        let provider_name = provider_name.clone();
        let session_id = session_id.clone();
        Box::pin(async move {
            let sid = session_id.as_deref();

            let provider = state
                .providers
                .iter()
                .find(|p| p.config.name == provider_name)
                .ok_or_else(|| format!("Provider '{provider_name}' not found"))?;

            let recalled = state
                .memory
                .recall(&prompt, 5, sid)
                .await
                .unwrap_or_default();

            let context_block = if recalled.is_empty() {
                String::new()
            } else {
                let mut ctx = String::from("\n--- Recalled Memories ---\n");
                for (i, entry) in recalled.iter().enumerate() {
                    ctx.push_str(&format!(
                        "{}. [{}] {}: {}\n",
                        i + 1, entry.category, entry.key, entry.content
                    ));
                }
                ctx.push_str("--- End ---\n\n");
                ctx
            };

            let augmented = format!("{context_block}User: {prompt}");
            let config = provider.config.clone();
            let response_text = call_provider(&config, &augmented).await?;

            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();

            let _ = state.memory.store(
                &format!("turn_{ts}_user"),
                &prompt,
                MemoryCategory::Conversation,
                sid,
            ).await;

            let _ = state.memory.store(
                &format!("turn_{ts}_assistant"),
                &response_text,
                MemoryCategory::Conversation,
                sid,
            ).await;

            Ok(response_text)
        })
    })
    .await
}

// ─── 4–10. Direct Memory Operations ────────────────────────

pub async fn memory_store(
    key: String,
    content: String,
    category: String,
    session_id: Option<String>,
) -> Result<(), String> {
    with_state(|state| {
        let key = key.clone();
        let content = content.clone();
        let category = category.clone();
        let session_id = session_id.clone();
        Box::pin(async move {
            state.memory.store(&key, &content, parse_category(&category), session_id.as_deref()).await
                .map_err(|e| format!("memory_store failed: {e}"))
        })
    })
    .await
}

pub async fn memory_recall(
    query: String,
    limit: u32,
    session_id: Option<String>,
) -> Result<Vec<MemoryEntryDto>, String> {
    with_state(|state| {
        let query = query.clone();
        let session_id = session_id.clone();
        Box::pin(async move {
            let entries = state.memory.recall(&query, limit as usize, session_id.as_deref()).await
                .map_err(|e| format!("memory_recall failed: {e}"))?;
            Ok(entries.into_iter().map(MemoryEntryDto::from).collect())
        })
    })
    .await
}

pub async fn memory_get(key: String) -> Result<Option<MemoryEntryDto>, String> {
    with_state(|state| {
        let key = key.clone();
        Box::pin(async move {
            let entry = state.memory.get(&key).await
                .map_err(|e| format!("memory_get failed: {e}"))?;
            Ok(entry.map(MemoryEntryDto::from))
        })
    })
    .await
}

pub async fn memory_list(
    category: Option<String>,
    session_id: Option<String>,
) -> Result<Vec<MemoryEntryDto>, String> {
    with_state(|state| {
        let category = category.clone();
        let session_id = session_id.clone();
        Box::pin(async move {
            let cat = parse_category_filter(&category);
            let entries = state.memory.list(cat.as_ref(), session_id.as_deref()).await
                .map_err(|e| format!("memory_list failed: {e}"))?;
            Ok(entries.into_iter().map(MemoryEntryDto::from).collect())
        })
    })
    .await
}

pub async fn memory_forget(key: String) -> Result<bool, String> {
    with_state(|state| {
        let key = key.clone();
        Box::pin(async move {
            state.memory.forget(&key).await
                .map_err(|e| format!("memory_forget failed: {e}"))
        })
    })
    .await
}

pub async fn memory_count() -> Result<u64, String> {
    with_state(|state| {
        Box::pin(async move {
            let c = state.memory.count().await
                .map_err(|e| format!("memory_count failed: {e}"))?;
            Ok(c as u64)
        })
    })
    .await
}

pub async fn memory_health() -> Result<bool, String> {
    with_state(|state| {
        Box::pin(async move { Ok(state.memory.health_check().await) })
    })
    .await
}

pub async fn memory_reindex() -> Result<u64, String> {
    // Reindexing is not available in the Memory trait for zeroclaw 0.1.7.
    // We return 0 for now to keep the FFI signature unchanged.
    Ok(0)
}

// ─── 11. Status ─────────────────────────────────────────────

pub async fn agent_status() -> Result<AgentStatus, String> {
    with_state(|state| {
        Box::pin(async move {
            let count = state.memory.count().await
                .map_err(|e| format!("status: {e}"))?;
            Ok(AgentStatus {
                initialized: true,
                memory_backend: state.memory.name().to_string(),
                data_dir: state.data_dir.display().to_string(),
                memory_count: count as u64,
                provider_count: state.providers.len() as u32,
                routing_strategy: strategy_name(&state.strategy).to_string(),
            })
        })
    })
    .await
}
