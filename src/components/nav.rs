use leptos::prelude::*;
use leptos_router::components::A;

use crate::app::AuthState;

#[component]
pub fn Nav() -> impl IntoView {
    let auth = use_context::<AuthState>().expect("AuthState missing");
    let auth2 = auth.clone();

    view! {
        <nav class="border-b border-border bg-panel">
            <div class="max-w-7xl mx-auto px-4 h-12 flex items-center justify-between">
                <div class="flex items-center gap-6">
                    <a href="/" class="font-semibold text-sm tracking-tight hover:text-white transition-colors">"Martingale"</a>
                    <Show when=move || auth.is_authenticated()>
                        <div class="flex gap-4">
                            <NavLink href="/app/portfolio">"Portfolio"</NavLink>
                            <NavLink href="/app/pricer">"Pricer"</NavLink>
                            <NavLink href="/app/scenarios">"Scenarios"</NavLink>
                            <NavLink href="/app/tax">"Taxes"</NavLink>
                        </div>
                    </Show>
                </div>
                <Show when=move || auth2.is_authenticated()>
                    <button
                        class="text-xs text-gray-500 hover:text-gray-300 transition-colors"
                        on:click=move |_| {
                            auth2.logout();
                            // Leave the SPA for the static landing page at the site root.
                            web_sys::window().unwrap().location().set_href("/").ok();
                        }
                    >
                        "Sign out"
                    </button>
                </Show>
            </div>
        </nav>
    }
}

#[component]
fn NavLink(href: &'static str, children: Children) -> impl IntoView {
    view! {
        <A href=href attr:class="text-xs text-gray-400 hover:text-gray-100 transition-colors">
            {children()}
        </A>
    }
}
