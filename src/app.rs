use gloo_storage::Storage as _;
use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos_router::{
    components::{Route, Router, Routes, A},
    path,
};
use wasm_bindgen_futures::spawn_local;

use crate::api::supabase::{self, AuthResponse};
use crate::components::nav::Nav;
use crate::store::MarketStore;
use crate::pages::{
    import::ImportPage, login::LoginPage, portfolio::PortfolioPage, pricer::PricerPage,
    scenarios::ScenariosPage, tax::TaxPage,
};

const REFRESH_INTERVAL_MS: u32 = 45 * 60 * 1000; // 45 minutes

/// Global auth state provided at root — consumed anywhere via use_context.
#[derive(Clone, Copy, Debug)]
pub struct AuthState {
    pub token: RwSignal<Option<String>>,
    pub user_id: RwSignal<Option<String>>,
    pub refresh_token: RwSignal<Option<String>>,
}

impl AuthState {
    fn load() -> Self {
        let token = gloo_storage::LocalStorage::get("sb_token").ok();
        let user_id = gloo_storage::LocalStorage::get("sb_user_id").ok();
        let refresh_token = gloo_storage::LocalStorage::get("sb_refresh_token").ok();
        Self {
            token: RwSignal::new(token),
            user_id: RwSignal::new(user_id),
            refresh_token: RwSignal::new(refresh_token),
        }
    }

    pub fn is_authenticated(&self) -> bool {
        self.token.get().is_some()
    }

    pub fn apply_session(&self, resp: AuthResponse) {
        gloo_storage::LocalStorage::set("sb_token", &resp.access_token).ok();
        gloo_storage::LocalStorage::set("sb_user_id", &resp.user.id).ok();
        gloo_storage::LocalStorage::set("sb_refresh_token", &resp.refresh_token).ok();
        self.token.set(Some(resp.access_token));
        self.user_id.set(Some(resp.user.id));
        self.refresh_token.set(Some(resp.refresh_token));
    }

    pub fn logout(&self) {
        gloo_storage::LocalStorage::delete("sb_token");
        gloo_storage::LocalStorage::delete("sb_user_id");
        gloo_storage::LocalStorage::delete("sb_refresh_token");
        self.token.set(None);
        self.user_id.set(None);
        self.refresh_token.set(None);
    }

    pub fn start_refresh_loop(&self) {
        let auth = *self;
        spawn_local(async move {
            loop {
                TimeoutFuture::new(REFRESH_INTERVAL_MS).await;
                let rt = match auth.refresh_token.get_untracked() {
                    Some(t) => t,
                    None => break,
                };
                match supabase::refresh_session(&rt).await {
                    Ok(resp) => auth.apply_session(resp),
                    Err(_) => {
                        auth.logout();
                        let win = web_sys::window().unwrap();
                        win.location().set_href(&format!("{}/login", crate::config::APP_BASE)).ok();
                        break;
                    }
                }
            }
        });
    }
}

#[component]
pub fn App() -> impl IntoView {
    let auth = AuthState::load();
    provide_context(auth);
    provide_context(MarketStore::new());

    // If we have a stored refresh token, immediately exchange it for a fresh
    // access token (handles returning after hours) then start the 45-min loop.
    if auth.refresh_token.get_untracked().is_some() {
        let auth = auth;
        spawn_local(async move {
            let rt = match auth.refresh_token.get_untracked() {
                Some(t) => t,
                None => return,
            };
            match supabase::refresh_session(&rt).await {
                Ok(resp) => {
                    auth.apply_session(resp);
                    auth.start_refresh_loop();
                }
                Err(_) => auth.logout(),
            }
        });
    }

    view! {
        <Router base=crate::config::APP_BASE>
            <Nav />
            <main class="max-w-7xl mx-auto px-4 py-8">
                <Routes fallback=|| view! { <NotFound /> }>
                    <Route path=path!("/") view=move || view! { <Redirect /> } />
                    <Route path=path!("/login") view=LoginPage />
                    <Route path=path!("/pricer") view=move || view! { <Protected><PricerPage /></Protected> } />
                    <Route path=path!("/portfolio") view=move || view! { <Protected><PortfolioPage /></Protected> } />
                    <Route path=path!("/import") view=move || view! { <Protected><ImportPage /></Protected> } />
                    <Route path=path!("/scenarios") view=move || view! { <Protected><ScenariosPage /></Protected> } />
                    <Route path=path!("/tax") view=move || view! { <Protected><TaxPage /></Protected> } />
                </Routes>
            </main>
        </Router>
    }
}

#[component]
fn Protected(children: ChildrenFn) -> impl IntoView {
    let auth = use_context::<AuthState>().expect("AuthState missing");
    view! {
        {move || if auth.is_authenticated() {
            children().into_any()
        } else {
            view! { <leptos_router::components::Redirect path="/login" /> }.into_any()
        }}
    }
}

#[component]
fn Redirect() -> impl IntoView {
    let auth = use_context::<AuthState>().expect("AuthState missing");
    view! {
        {move || {
            if auth.is_authenticated() {
                view! { <leptos_router::components::Redirect path="/portfolio" /> }.into_any()
            } else {
                view! { <leptos_router::components::Redirect path="/login" /> }.into_any()
            }
        }}
    }
}

#[component]
fn NotFound() -> impl IntoView {
    view! {
        <div class="text-center mt-32 text-gray-400">
            <p class="text-4xl mb-2">"404"</p>
            <p class="text-sm mb-4 font-sans">"There's nothing at this address."</p>
            <A href="/" attr:class="text-blue-400 hover:underline">"Back to your portfolio"</A>
        </div>
    }
}
