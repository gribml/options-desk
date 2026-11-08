use leptos::prelude::*;

use crate::{app::AuthState, api::supabase};
use wasm_bindgen_futures::spawn_local;

#[component]
pub fn SettingsPage() -> impl IntoView {
    let auth = use_context::<AuthState>().expect("AuthState missing");
    let hide_closed = RwSignal::new(false);
    let busy = RwSignal::new(true);
    let error = RwSignal::new(None::<String>);

    Effect::new(move |_| {
        if let (Some(token), Some(user_id)) = (auth.token.get(), auth.user_id.get()) {
            busy.set(true);
            spawn_local(async move {
                match supabase::fetch_user_settings(&token, &user_id).await {
                    Ok(settings) => hide_closed.set(settings.hide_closed_positions),
                    Err(e) => error.set(Some(e)),
                }
                busy.set(false);
            });
        }
    });

    view! {
        <div class="space-y-6">
            <div>
                <h1 class="text-xl font-semibold">"Settings"</h1>
                <p class="text-xs text-gray-500 mt-1 font-sans">"Your display preferences."</p>
            </div>
            <label class="flex items-start gap-3 p-4 rounded border border-border bg-panel cursor-pointer">
                <input
                    type="checkbox"
                    class="mt-0.5"
                    prop:checked=move || hide_closed.get()
                    prop:disabled=move || busy.get()
                    on:change=move |ev| {
                        let value = event_target_checked(&ev);
                        if let (Some(token), Some(user_id)) = (auth.token.get_untracked(), auth.user_id.get_untracked()) {
                            busy.set(true);
                            error.set(None);
                            spawn_local(async move {
                                match supabase::save_user_settings(&token, &user_id, value).await {
                                    Ok(()) => hide_closed.set(value),
                                    Err(e) => error.set(Some(e)),
                                }
                                busy.set(false);
                            });
                        }
                    }
                />
                <span>
                    <span class="block text-sm">"Hide fully closed positions"</span>
                    <span class="block text-xs text-gray-500 mt-1 font-sans">
                        "Keep their trades and realized gains; only hide their portfolio rows."
                    </span>
                </span>
            </label>
            {move || error.get().map(|e| view! { <p class="text-sm text-red-400">{e}</p> })}
        </div>
    }
}
