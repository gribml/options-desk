use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::api::supabase;
use crate::app::AuthState;

#[component]
pub fn LoginPage() -> impl IntoView {
    let auth = use_context::<AuthState>().expect("AuthState missing");
    let email = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let error = RwSignal::new(Option::<String>::None);
    let loading = RwSignal::new(false);

    let on_submit = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        let e = email.get();
        let p = password.get();
        if e.is_empty() || p.is_empty() {
            error.set(Some("Email and password required.".into()));
            return;
        }
        error.set(None);
        loading.set(true);

        let auth = auth.clone();
        spawn_local(async move {
            match supabase::login(&e, &p).await {
                Ok(resp) => {
                    auth.apply_session(resp);
                    auth.start_refresh_loop();
                    let win = web_sys::window().unwrap();
                    win.location().set_href(&format!("{}/portfolio", crate::config::APP_BASE)).ok();
                }
                Err(msg) => {
                    error.set(Some(msg));
                    loading.set(false);
                }
            }
        });
    };

    view! {
        <div class="min-h-[80vh] flex items-center justify-center">
            <div class="bg-panel border border-border rounded-xl p-8 w-full max-w-sm">
                <h1 class="text-2xl font-semibold mb-6 text-center">"Strike"</h1>

                <form on:submit=on_submit class="space-y-4">
                    <div>
                        <label class="block text-sm text-gray-400 mb-1">"Email"</label>
                        <input
                            type="email"
                            class="w-full bg-surface border border-border rounded px-3 py-2 text-sm focus:outline-none focus:border-blue-500"
                            prop:value=move || email.get()
                            on:input=move |ev| email.set(event_target_value(&ev))
                            placeholder="you@example.com"
                        />
                    </div>
                    <div>
                        <label class="block text-sm text-gray-400 mb-1">"Password"</label>
                        <input
                            type="password"
                            class="w-full bg-surface border border-border rounded px-3 py-2 text-sm focus:outline-none focus:border-blue-500"
                            prop:value=move || password.get()
                            on:input=move |ev| password.set(event_target_value(&ev))
                            placeholder="••••••••"
                        />
                    </div>
                    {move || error.get().map(|msg| view! {
                        <p class="text-red-400 text-sm">{msg}</p>
                    })}
                    <button
                        type="submit"
                        class="w-full bg-blue-600 hover:bg-blue-500 disabled:opacity-50 rounded px-4 py-2 text-sm font-medium transition-colors"
                        prop:disabled=move || loading.get()
                    >
                        {move || if loading.get() { "Signing in…" } else { "Sign in" }}
                    </button>
                </form>
            </div>
        </div>
    }
}
