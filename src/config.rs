pub const SUPABASE_URL: &str = match option_env!("SUPABASE_URL") {
    Some(v) => v,
    None => "https://placeholder.supabase.co",
};

pub const SUPABASE_ANON_KEY: &str = match option_env!("SUPABASE_ANON_KEY") {
    Some(v) => v,
    None => "placeholder-anon-key",
};

pub const WORKER_URL: &str = match option_env!("WORKER_URL") {
    Some(v) => v,
    None => "http://localhost:8787",
};

/// URL path the WASM app is mounted under. The static landing page lives at `/`;
/// everything the router serves lives beneath this base. Keep in sync with
/// `public_url` in `Trunk.toml` and the `_redirects` SPA fallback.
pub const APP_BASE: &str = "/app";
