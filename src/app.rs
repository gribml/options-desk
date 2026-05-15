use gloo_storage::Storage as _;
use leptos::prelude::*;
use leptos_router::{
    components::{Route, Router, Routes, A},
    path,
};

use crate::components::nav::Nav;
use crate::pages::{
    login::LoginPage, portfolio::PortfolioPage, pricer::PricerPage, scenarios::ScenariosPage,
};

/// Global auth state provided at root — consumed anywhere via use_context.
#[derive(Clone, Copy, Debug)]
pub struct AuthState {
    pub token: RwSignal<Option<String>>,
    pub user_id: RwSignal<Option<String>>,
}

impl AuthState {
    fn load() -> Self {
        let token = gloo_storage::LocalStorage::get("sb_token").ok();
        let user_id = gloo_storage::LocalStorage::get("sb_user_id").ok();
        Self {
            token: RwSignal::new(token),
            user_id: RwSignal::new(user_id),
        }
    }

    pub fn is_authenticated(&self) -> bool {
        self.token.get().is_some()
    }

    pub fn logout(&self) {
        gloo_storage::LocalStorage::delete("sb_token");
        gloo_storage::LocalStorage::delete("sb_user_id");
        self.token.set(None);
        self.user_id.set(None);
    }
}

#[component]
pub fn App() -> impl IntoView {
    let auth = AuthState::load();
    provide_context(auth);

    view! {
        <Router>
            <Nav />
            <main class="max-w-7xl mx-auto px-4 py-8">
                <Routes fallback=|| view! { <NotFound /> }>
                    <Route path=path!("/") view=move || view! { <Redirect /> } />
                    <Route path=path!("/login") view=LoginPage />
                    <Route path=path!("/pricer") view=PricerPage />
                    <Route path=path!("/portfolio") view=PortfolioPage />
                    <Route path=path!("/scenarios") view=ScenariosPage />
                </Routes>
            </main>
        </Router>
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
            <p class="text-4xl mb-4">"404"</p>
            <A href="/" attr:class="text-blue-400 hover:underline">"Go home"</A>
        </div>
    }
}
