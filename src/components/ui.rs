//! Shared presentation primitives for a walk-up-usable UI.
//!
//! Three ideas run through these:
//!   - `Info` puts a plain-English definition one hover/tap away from any term,
//!     so jargon can stay on screen without gatekeeping the page.
//!   - `Disclosure` keeps expert detail reachable but collapsed, so the default
//!     view stays quiet.
//!   - `EmptyState` / `Hint` / `Callout` carry prose, and deliberately switch to
//!     `font-sans` — the app is globally monospaced, which reads as a terminal.
//!     Numbers stay mono; sentences don't.

use leptos::prelude::*;

use crate::glossary;

/// A "?" affordance that reveals a plain-English definition of `term` on hover
/// (desktop) or tap (touch). Unknown keys render nothing rather than a dead dot.
#[component]
pub fn Info(
    term: &'static str,
    /// Anchor the popover's right edge to the dot instead of centring it. Use for
    /// terms sitting near the right edge of the page, where a centred popover
    /// would overflow a narrow viewport.
    #[prop(default = false)]
    align_end: bool,
) -> impl IntoView {
    let Some(def) = glossary::lookup(term) else {
        return ().into_any();
    };
    let open = RwSignal::new(false);

    // One element, shown by class rather than by branch: `group-hover` covers
    // mouse users and the signal covers taps, without rendering two panels.
    let anchor = if align_end { "right-0" } else { "left-1/2 -translate-x-1/2" };
    let panel_cls = move || {
        format!(
            "absolute z-50 {anchor} top-full mt-2 w-56 sm:w-64 max-w-[calc(100vw-2rem)] \
             rounded-lg border border-border bg-panel p-3 text-left shadow-xl \
             pointer-events-none normal-case tracking-normal {}",
            if open.get() { "block" } else { "hidden group-hover:block" }
        )
    };

    view! {
        <span class="group relative inline-flex align-middle">
            <button
                type="button"
                class="w-3.5 h-3.5 inline-flex items-center justify-center rounded-full border \
                       border-gray-600 text-gray-500 text-[9px] leading-none font-sans \
                       hover:border-blue-400 hover:text-blue-300 transition-colors"
                aria-label=format!("What is {}?", def.title)
                on:click=move |ev: web_sys::MouseEvent| {
                    ev.stop_propagation();
                    ev.prevent_default();
                    open.update(|v| *v = !*v);
                }
                on:blur=move |_| open.set(false)
            >
                "?"
            </button>
            <span class=panel_cls>
                <span class="block text-xs font-semibold text-gray-200 mb-1 font-sans">
                    {def.title}
                </span>
                <span class="block text-xs leading-relaxed text-gray-400 font-sans">
                    {def.body}
                </span>
            </span>
        </span>
    }
    .into_any()
}

/// A field label with an optional definition attached.
#[component]
pub fn Label(
    #[prop(into)] text: String,
    #[prop(optional_no_strip)] term: Option<&'static str>,
) -> impl IntoView {
    view! {
        <label class="flex items-center gap-1.5 text-xs text-gray-400 mb-1 font-sans">
            {text}
            {term.map(|t| view! { <Info term=t /> })}
        </label>
    }
}

/// A collapsed section holding detail that would otherwise clutter the page.
/// Children are only built while open, so charts and effects inside stay idle.
#[component]
pub fn Disclosure(
    #[prop(into)] summary: String,
    /// One line explaining what's inside, shown once expanded.
    #[prop(optional, into)]
    detail: Option<String>,
    #[prop(default = false)] open: bool,
    children: ChildrenFn,
) -> impl IntoView {
    let expanded = RwSignal::new(open);
    view! {
        <div class="space-y-2">
            <button
                type="button"
                class="flex items-center gap-1.5 text-xs text-gray-500 hover:text-gray-300 \
                       transition-colors font-sans"
                on:click=move |_| expanded.update(|v| *v = !*v)
            >
                <span class="inline-block w-2 text-[10px]">
                    {move || if expanded.get() { "▾" } else { "▸" }}
                </span>
                {summary}
            </button>
            {move || {
                expanded
                    .get()
                    .then(|| {
                        view! {
                            <div class="space-y-3">
                                {detail
                                    .clone()
                                    .map(|d| {
                                        view! {
                                            <p class="text-xs text-gray-500 leading-relaxed font-sans">
                                                {d}
                                            </p>
                                        }
                                    })}
                                {children()}
                            </div>
                        }
                    })
            }}
        </div>
    }
}

/// The zero-state for a page: says what this screen is for and what to do next.
/// `children` holds the call-to-action.
#[component]
pub fn EmptyState(
    #[prop(into)] title: String,
    #[prop(into)] body: String,
    children: Children,
) -> impl IntoView {
    view! {
        <div class="bg-panel border border-border rounded-xl px-6 py-10 text-center">
            <p class="text-base font-semibold text-gray-200 mb-2 font-sans">{title}</p>
            <p class="text-sm text-gray-400 leading-relaxed max-w-md mx-auto mb-5 font-sans">
                {body}
            </p>
            <div class="flex flex-wrap gap-2 justify-center">{children()}</div>
        </div>
    }
}

/// A muted one-or-two-line explanation sitting under a heading or control.
#[component]
pub fn Hint(children: Children) -> impl IntoView {
    view! {
        <p class="text-xs text-gray-500 leading-relaxed font-sans">{children()}</p>
    }
}

/// Tone for a `Callout`.
#[derive(Clone, Copy, PartialEq)]
pub enum Tone {
    Info,
    Warn,
}

/// A boxed note that needs to be read — a caveat, a prerequisite, a warning.
#[component]
pub fn Callout(#[prop(default = Tone::Info)] tone: Tone, children: Children) -> impl IntoView {
    let cls = match tone {
        Tone::Info => "border-blue-900/70 bg-blue-950/30 text-blue-200/90",
        Tone::Warn => "border-amber-900 bg-amber-950/40 text-amber-300",
    };
    view! {
        <div class=format!(
            "rounded-lg border px-3 py-2 text-xs leading-relaxed font-sans {}",
            cls,
        )>{children()}</div>
    }
}

/// A headline number with its label and an optional definition — the unit the
/// summary rows are built from.
#[component]
pub fn Stat(
    #[prop(into)] label: String,
    #[prop(optional)] term: Option<&'static str>,
    #[prop(optional, into)] value_class: Option<String>,
    children: Children,
) -> impl IntoView {
    view! {
        <div>
            <div class="flex items-center gap-1.5 mb-1">
                <span class="text-xs text-gray-400 font-sans">{label}</span>
                {term.map(|t| view! { <Info term=t /> })}
            </div>
            <p class=value_class.unwrap_or_else(|| "text-lg font-semibold".to_string())>
                {children()}
            </p>
        </div>
    }
}
