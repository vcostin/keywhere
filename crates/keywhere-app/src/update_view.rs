//! Update UI: the "new version" toast and the Settings → Updates section.
//! State lives in [`crate::update`]; these only draw it.

use gpui::{div, prelude::*, px, svg, AnyElement, App, FontWeight, SharedString};

use crate::i18n::{t, tf};
use crate::theme::*;
use crate::update::{self, Install, Status, UpdateError};

fn button(id: &'static str, label: SharedString, primary: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(28.))
        .px_3()
        .flex()
        .items_center()
        .rounded(px(7.))
        .cursor_pointer()
        .text_size(px(12.))
        .when(primary, |d| {
            d.bg(c(ACCENT))
                .text_color(c(ACCENT_INK))
                .font_weight(FontWeight::SEMIBOLD)
        })
        .when(!primary, |d| {
            d.border_1()
                .border_color(c(BORDER))
                .bg(c(RAISED))
                .text_color(c(TEXT_SOFT))
                .hover(|d| d.bg(c(HOVER)))
        })
        .child(label)
}

fn error_text(e: &UpdateError) -> SharedString {
    match e {
        UpdateError::Network(_) => t("Couldn't reach GitHub. Check your connection.").into(),
        UpdateError::Verify(d) => tf(
            "The update failed verification and was not installed: {e}",
            &[("e", t(d))],
        )
        .into(),
        UpdateError::Install(d) => tf("Couldn't install the update: {e}", &[("e", d)]).into(),
    }
}

/// The action for an available release: install in place, or (for installs
/// we don't own, or no build for this system) open the release page.
fn primary_action(rel: &update::Release) -> gpui::Stateful<gpui::Div> {
    let can_install =
        rel.asset.is_some() && matches!(update::install_method(), Install::AppImage(_));
    if can_install {
        button("update-install", t("Update and restart").into(), true)
            .on_click(|_, _, cx| crate::install_update(cx))
    } else {
        let url = rel.notes_url.clone();
        button("update-page", t("Download").into(), true)
            .on_click(move |_, _, cx| cx.open_url(&url))
    }
}

/// Bottom-right card while an update is available, downloading or failed.
pub fn toast(_cx: &App) -> Option<AnyElement> {
    let status = update::status();
    let (msg, actions): (SharedString, Vec<AnyElement>) = match &status {
        Status::Available(rel) if update::banner().is_some() => {
            let notes = rel.notes_url.clone();
            (
                tf("Keywhere {v} is available.", &[("v", &rel.version.to_string())]).into(),
                vec![
                    primary_action(rel).into_any_element(),
                    button("update-notes", t("What's new").into(), false)
                        .on_click(move |_, _, cx| cx.open_url(&notes))
                        .into_any_element(),
                    button("update-later", t("Later").into(), false)
                        .on_click(|_, window, _| {
                            update::dismiss();
                            window.refresh();
                        })
                        .into_any_element(),
                ],
            )
        }
        Status::Downloading { version, percent } => (
            tf(
                "Downloading Keywhere {v}… {p}%",
                &[("v", &version.to_string()), ("p", &percent.to_string())],
            )
            .into(),
            vec![],
        ),
        Status::Failed(e @ (UpdateError::Verify(_) | UpdateError::Install(_))) => (
            error_text(e),
            vec![button("update-dismiss", t("Dismiss").into(), false)
                .on_click(|_, window, _| {
                    update::clear();
                    window.refresh();
                })
                .into_any_element()],
        ),
        Status::Idle if crate::settings::get().check_updates.is_none() => (
            t("Check GitHub for new Keywhere versions at startup? Nothing about your vaults is sent.").into(),
            vec![
                button("update-ask-yes", t("Check for updates").into(), true)
                    .on_click(|_, window, cx| {
                        crate::settings::update(|s| s.check_updates = Some(true));
                        update::start_check(false, cx);
                        window.refresh();
                    })
                    .into_any_element(),
                button("update-ask-no", t("Not now").into(), false)
                    .on_click(|_, window, _| {
                        crate::settings::update(|s| s.check_updates = Some(false));
                        window.refresh();
                    })
                    .into_any_element(),
            ],
        ),
        _ => return None,
    };
    let col = if matches!(status, Status::Failed(_)) {
        WARN
    } else {
        ACCENT
    };
    Some(
        div()
            .id("update-toast")
            .absolute()
            .bottom(px(44.))
            .right(px(16.))
            .w(px(340.))
            .max_w_full()
            .p_3()
            .rounded(px(10.))
            .bg(c(PANEL))
            .border_1()
            .border_color(ca(col, 0x88))
            .shadow_lg()
            .occlude()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .child(
                        svg()
                            .path(if matches!(status, Status::Failed(_)) {
                                "icons/warning.svg"
                            } else {
                                "icons/shield.svg"
                            })
                            .size(px(14.))
                            .mt(px(1.))
                            .flex_none()
                            .text_color(c(col)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.5))
                            .text_color(c(TEXT))
                            .child(msg),
                    ),
            )
            .when(!actions.is_empty(), |d| {
                d.child(div().flex().flex_wrap().gap_2().children(actions))
            })
            .into_any_element(),
    )
}

/// Settings → Updates: auto-check toggle, "Check now", status, install.
pub fn settings_section(
    check_updates: bool,
    on_toggle: impl Fn(&mut gpui::Window) + 'static,
) -> gpui::Div {
    let status = update::status();
    let busy = matches!(status, Status::Checking | Status::Downloading { .. });
    let line: Option<SharedString> = match &status {
        Status::Idle => None,
        Status::Checking => Some(t("Checking…").into()),
        Status::UpToDate => Some(
            tf(
                "Keywhere {v} is up to date.",
                &[("v", &update::current_version().to_string())],
            )
            .into(),
        ),
        Status::Available(rel) => Some(
            tf(
                "Keywhere {v} is available.",
                &[("v", &rel.version.to_string())],
            )
            .into(),
        ),
        Status::Downloading { version, percent } => Some(
            tf(
                "Downloading Keywhere {v}… {p}%",
                &[("v", &version.to_string()), ("p", &percent.to_string())],
            )
            .into(),
        ),
        Status::Failed(e) => Some(error_text(e)),
    };
    let toggle = div()
        .id("updates-auto")
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .on_click(move |_, window, _| on_toggle(window))
        .child(
            div()
                .size(px(16.))
                .rounded(px(4.))
                .border_1()
                .border_color(c(if check_updates { 0x4E7BE0 } else { BORDER }))
                .bg(if check_updates {
                    ca(FOCUS, 0x2e)
                } else {
                    ca(BG, 0)
                })
                .flex()
                .items_center()
                .justify_center()
                .when(check_updates, |d| {
                    d.child(
                        svg()
                            .path("icons/check.svg")
                            .size(px(11.))
                            .text_color(c(0xCFDCFF)),
                    )
                }),
        )
        .child(
            div()
                .text_size(px(12.5))
                .text_color(c(TEXT_SOFT))
                .child(t("Check for updates at startup")),
        );
    let mut actions = div().flex().flex_wrap().items_center().gap_2().child(
        button(
            "updates-check",
            t(if matches!(status, Status::Checking) {
                "Checking…"
            } else {
                "Check now"
            })
            .into(),
            false,
        )
        .when(!busy, |d| {
            d.on_click(|_, _, cx| update::start_check(true, cx))
        })
        .when(busy, |d| d.opacity(0.5).cursor_default()),
    );
    if let Status::Available(rel) = &status {
        actions = actions.child(primary_action(rel));
    }
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(toggle)
        .child(actions)
        .when_some(line, |d, l| {
            d.child(
                div()
                    .text_size(px(12.))
                    .text_color(c(if matches!(status, Status::Failed(_)) { WARN } else { MUTED }))
                    .child(l),
            )
        })
        .child(
            div()
                .text_size(px(11.))
                .text_color(c(DIM))
                .child(t("Checks ask GitHub for the latest release. Updates are verified with Keywhere's signing key before they are installed. Nothing about your vaults is sent.")),
        )
}
