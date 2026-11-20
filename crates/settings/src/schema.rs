//! Schema version and table definitions.

/// Version written to `schema_meta`. Only this version opens; any other is refused as `Corrupt`
/// (old development databases are refused, never silently migrated or deleted).
pub(crate) const SCHEMA_VERSION: i64 = 7;

/// Every table of [`SCHEMA_VERSION`], created in order on an empty database.
pub(crate) const TABLES: &[&str] = &[
    "CREATE TABLE schema_meta (version INTEGER NOT NULL)",
    "CREATE TABLE providers (
        id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, default_protocol TEXT NOT NULL,
        headers TEXT NOT NULL,
        access TEXT NOT NULL, enabled INTEGER NOT NULL, sort INTEGER NOT NULL,
        revision INTEGER NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
        preset_id TEXT UNIQUE)",
    "CREATE TABLE provider_endpoints (
        provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
        protocol TEXT NOT NULL, base_url TEXT NOT NULL, PRIMARY KEY (provider_id, protocol))",
    "CREATE TABLE provider_models (
        provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
        model_id TEXT NOT NULL, name TEXT NOT NULL, protocols TEXT NOT NULL,
        context_window INTEGER, max_tokens INTEGER, vision INTEGER NOT NULL, reasoning INTEGER NOT NULL,
        tools INTEGER NOT NULL, efforts TEXT NOT NULL, enabled INTEGER NOT NULL, sort INTEGER NOT NULL,
        model_type TEXT NOT NULL DEFAULT 'chat',
        PRIMARY KEY (provider_id, model_id))",
    "CREATE TABLE provider_keys (
        id TEXT PRIMARY KEY, provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
        key TEXT NOT NULL, label TEXT, enabled INTEGER NOT NULL, sort INTEGER NOT NULL,
        UNIQUE (provider_id, key))",
    "CREATE TABLE model_prefs (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
    "CREATE TABLE hidden_models (
        provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE, model_id TEXT NOT NULL,
        PRIMARY KEY (provider_id, model_id))",
    "CREATE TABLE mcp_servers (
        id TEXT PRIMARY KEY, name TEXT NOT NULL,
        project_id TEXT NOT NULL,
        transport TEXT NOT NULL,
        enabled INTEGER NOT NULL, description TEXT, revision INTEGER NOT NULL, sort INTEGER NOT NULL,
        UNIQUE (project_id, name))",
    "CREATE TABLE mcp_overrides (
        project_id TEXT NOT NULL, server_id TEXT NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
        enabled INTEGER NOT NULL, PRIMARY KEY (project_id, server_id))",
    "CREATE TABLE app_prefs (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
    "CREATE TABLE project_prefs (
        project_id TEXT PRIMARY KEY, backend TEXT,
        model_provider TEXT REFERENCES providers(id) ON DELETE SET NULL, model_id TEXT,
        effort TEXT, mode TEXT NOT NULL, view TEXT, revision INTEGER NOT NULL,
        CHECK ((model_provider IS NULL) = (model_id IS NULL)))",
    "CREATE TRIGGER clear_project_models BEFORE DELETE ON providers BEGIN
        UPDATE project_prefs SET model_provider = NULL, model_id = NULL, revision = revision + 1
        WHERE model_provider = OLD.id; END",
    "CREATE TABLE asset_keys (
        provider TEXT PRIMARY KEY, key TEXT NOT NULL, revision INTEGER NOT NULL)",
    "CREATE TABLE chat_bindings (
        chat_id TEXT PRIMARY KEY, project_id TEXT NOT NULL, backend TEXT NOT NULL,
        native_session TEXT NOT NULL)",
    "CREATE INDEX chat_bindings_project ON chat_bindings(project_id)",
    "CREATE TABLE builder_chats (
        project_id TEXT PRIMARY KEY, chat_id TEXT NOT NULL UNIQUE, backend TEXT NOT NULL)",
];
