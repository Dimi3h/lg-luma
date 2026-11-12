use crate::{
    model::{App, Input, InputMode, Preferences, demo_inventory},
    platform::{normalize_apps, normalize_inputs},
    state::{Direction, Effect, Overlay, Page, State, Target},
    storage, view,
};
use serde_json::json;
use std::{collections::BTreeMap, path::PathBuf};

#[path = "recents-tests.rs"]
mod recent_tests;

#[path = "wallpapers-ui-tests.rs"]
mod wallpaper_tests;

#[path = "input-refresh-tests.rs"]
mod input_refresh_tests;

fn ready() -> State {
    let mut state = State::new(Preferences::default());
    state.set_inventory(demo_inventory(), None);
    state
}

#[test]
fn home_relaunch_returns_from_scrolled_home_and_settings() {
    for page in [Page::Home, Page::Settings] {
        let mut state = ready();
        state.open(page);
        state.scroll = 400.;
        state.subpage = Some(crate::state::Subpage::Clock);
        state.pointer = true;
        state.pressed = Some(Target::SettingsIcon);
        state.dirty = false;

        assert!(state.return_home().is_empty());
        assert_eq!(state.page, Page::Home);
        // The grid opens on the first favourite.
        assert_eq!(state.focus, Target::App("netflix".into()));
        assert_eq!(state.scroll, 0.);
        assert!(state.subpage.is_none());
        assert!(!state.pointer);
        assert!(state.pressed.is_none());
        assert!(state.dirty);
        assert!(state.return_home().is_empty());
        assert!(state.overlay.is_none(), "Home again must not open Recents");
    }
}

#[test]
fn home_relaunch_dismisses_overlays_without_cancelling_mutations() {
    for overlay in [
        Overlay::Options {
            id: "test.app".into(),
            restore: Target::SettingsIcon,
        },
        Overlay::Delete {
            id: "test.app".into(),
            restore: Target::SettingsIcon,
        },
        Overlay::WallpaperUpload,
        Overlay::BackgroundOptions {
            id: "dunes".into(),
            restore: Target::Sidebar(1),
        },
        Overlay::BackgroundDelete {
            id: "dunes".into(),
            restore: Target::Sidebar(1),
        },
    ] {
        let mut state = ready();
        state.open(crate::state::Page::Settings);
        state.overlay = Some(overlay);
        state.wallpaper_upload = Some(Err("test".into()));
        state.uninstalling = Some("test.app".into());
        state.deleting_wallpaper = Some("dunes".into());
        state.moving = Some("test.app".into());

        assert!(state.return_home().is_empty());
        assert_eq!(state.page, Page::Home);
        assert!(state.overlay.is_none());
        assert!(state.wallpaper_upload.is_none());
        assert!(state.moving.is_none());
        assert_eq!(state.uninstalling.as_deref(), Some("test.app"));
        assert_eq!(state.deleting_wallpaper.as_deref(), Some("dunes"));
    }
}

#[test]
fn home_relaunch_commits_keyboard_edits_before_clearing_the_subpage() {
    for system in [false, true] {
        let mut state = ready();
        state.open(crate::state::Page::Settings);
        state.subpage = Some(crate::state::Subpage::Input("HDMI_1".into()));
        state.open_keyboard(crate::state::TextField::InputName);
        if system {
            state.overlay = Some(Overlay::SystemKeyboard);
        }
        state.text("Console");

        assert_eq!(state.return_home(), vec![Effect::Save]);
        assert_eq!(
            state
                .preferences
                .input_names
                .get("HDMI_1")
                .map(String::as_str),
            Some("Console")
        );
        assert_eq!(state.page, Page::Home);
        assert_eq!(state.focus, Target::App("netflix".into()));
        assert!(!state.text_input_active());
        assert!(state.subpage.is_none());
    }
}

#[test]
fn home_relaunch_dismisses_recents_and_ignores_its_pending_reply() {
    let mut state = ready();
    state.open(crate::state::Page::Home);
    state.open_recents();
    state.recents.visibility = 1.;
    state.recents.closing_focus = Some(Target::RecentEmpty);
    assert!(state.recents.pending.is_some());
    let generation = state.recents.generation;
    let command = state.recents.pending.clone().unwrap();

    assert!(state.return_home().is_empty());
    assert_eq!(state.page, Page::Home);
    assert!(state.overlay.is_none());
    assert!(state.recents.pending.is_none());
    assert_eq!(state.recents.visibility, 0.);
    assert!(state.recents.closing_focus.is_none());
    assert!(
        state
            .recent_response(generation, command, Ok(Some(vec![])))
            .is_empty()
    );
    assert_eq!(state.focus, Target::App("netflix".into()));
    assert!(state.overlay.is_none());
}

fn scene(state: &State) -> view::Scene {
    let assets = view::Assets {
        root: PathBuf::new(),
        version: "test".into(),
        metrics: BTreeMap::new(),
        artwork: crate::app_artwork::Service::new(false),
    };
    view::scene(state, &assets, time::OffsetDateTime::UNIX_EPOCH)
}

fn controls(state: &State) -> Vec<crate::state::Control> {
    scene(state).controls
}

/// Centres of full-width settings rows drawn with the opaque white focused surface. Focus
/// grows a row around its centre, so the centre (not its top) matches the control.
fn highlighted_rows(scene: &view::Scene) -> Vec<f32> {
    scene
        .draw
        .iter()
        .filter_map(|draw| match draw {
            crate::renderer::Draw::RoundedGradient(rect, _, top, _)
                if rect.w > view::SETTINGS_W - 1. && top[3] == 1. && top[0] == 1. =>
            {
                Some(rect.center().1)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn the_main_grid_scrolls_vertically_and_nothing_is_hit_beneath_the_clock() {
    let mut state = ready();
    let before = controls(&state);
    let netflix = before
        .iter()
        .find(|control| control.target == Target::App("netflix".into()))
        .unwrap()
        .rect;
    let (x, y) = netflix.center();
    assert_eq!(
        state.hit_test(&before, x, y),
        Some(Target::App("netflix".into()))
    );
    // Wheel scrolling moves the page (and reports it, cancelling a pending press).
    assert!(state.wheel_scroll(&before, Some((x, y)), 0, -1));
    assert_eq!(state.scroll, 90.);
    let after = controls(&state);
    let moved = after
        .iter()
        .find(|control| control.target == Target::App("netflix".into()))
        .unwrap()
        .rect;
    assert_eq!(netflix.y - moved.y, 90.);
    // Pointer focus never scrolls the clicked card.
    state.pointer_focus(Target::App("netflix".into()));
    assert_eq!(state.scroll, 90.);
    let card = crate::state::Control {
        target: Target::App("sample".into()),
        rect: crate::state::Rect::new(0., 40., 1920., 1100.),
        scrollable: true,
    };
    let cards = [card];
    assert!(
        state
            .hit_test(&cards, 100., crate::state::MAIN_TOP - 1.)
            .is_none()
    );
    assert_eq!(
        state.hit_test(&cards, 100., crate::state::MAIN_TOP + 1.),
        Some(Target::App("sample".into()))
    );
    assert!(state.hit_test(&cards, 100., 1080.).is_none());
    // The grid scrolls until its last row ends at the bottom margin.
    state.scroll = 0.;
    assert!(state.wheel_scroll(&cards, Some((100., 600.)), 0, -100));
    let viewport = state.content_viewport();
    let limit =
        cards[0].rect.y + cards[0].rect.h - (viewport.y + viewport.h - crate::view::SCREEN_MARGIN);
    assert_eq!(state.scroll, limit);
}

#[test]
fn remote_routes_through_the_shelf_grid_and_settings() {
    let mut state = ready();
    state.open(Page::Home);
    assert_eq!(state.focus, Target::App("netflix".into()));
    // Above the first row is the shelf's weather button (no location yet).
    state.direction(Direction::Up, &controls(&state));
    assert_eq!(state.focus, Target::WeatherConfigure);
    // Nothing above it; Right reaches the top bar's icons: Apps, then Settings.
    state.direction(Direction::Up, &controls(&state));
    assert_eq!(state.focus, Target::WeatherConfigure);
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.focus, Target::StoreIcon);
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.focus, Target::SettingsIcon);
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.focus, Target::SettingsIcon);
    // Down from Settings enters the right-hand column; Up from there returns.
    state.direction(Direction::Down, &controls(&state));
    let grid = state.grid();
    assert_eq!(state.focus, grid[5]);
    state.direction(Direction::Up, &controls(&state));
    assert_eq!(state.focus, Target::SettingsIcon);
    state.focus = grid[4].clone();
    state.direction(Direction::Up, &controls(&state));
    assert_eq!(state.focus, Target::StoreIcon);
    state.direction(Direction::Left, &controls(&state));
    assert_eq!(
        state.focus,
        Target::WeatherConfigure,
        "Left from the bar returns to the shelf"
    );
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::App("netflix".into()));
    // LG's app store is an icon, never a card.
    assert!(!grid.contains(&Target::App(crate::state::STORE_ID.into())));
    assert!(!grid.contains(&Target::SettingsIcon));
    state.focus = Target::StoreIcon;
    assert_eq!(
        state.activate(),
        vec![Effect::Launch(crate::state::STORE_ID.into())]
    );
    // Netflix is the first card now that there is no Search card.
    state.focus = Target::App("netflix".into());
    state.direction(Direction::Left, &controls(&state));
    assert_eq!(
        state.focus,
        Target::App("netflix".into()),
        "Rows do not wrap"
    );
    // The Settings icon opens Settings; Back from its sidebar returns to the icon.
    state.focus = Target::SettingsIcon;
    assert!(state.activate().is_empty());
    assert_eq!(state.page, Page::Settings);
    assert_eq!(state.focus, Target::Sidebar(0));
    state.activate();
    assert_eq!(state.focus, Target::Setting(0));
    assert!(state.back().is_empty());
    assert_eq!(state.focus, Target::Sidebar(0));
    state.back();
    assert_eq!(state.page, Page::Home);
    assert_eq!(state.focus, Target::SettingsIcon);
    assert_eq!(
        state.back(),
        vec![Effect::Recent {
            generation: 1,
            command: crate::recents::Command::List
        }]
    );
    assert!(state.recents_open());
}

#[test]
fn up_and_down_follow_grid_indices_including_offscreen_rows() {
    let mut state = ready();
    let grid = state.grid();
    assert_eq!(grid.first(), Some(&Target::App("netflix".into())));
    assert!(
        matches!(grid.last(), Some(Target::App(_))),
        "No Settings card"
    );
    for index in [1, 7, 13] {
        state.focus = grid[index].clone();
        state.direction(Direction::Down, &controls(&state));
        assert_eq!(state.focus, grid[index + 6]);
        state.direction(Direction::Up, &controls(&state));
        assert_eq!(state.focus, grid[index]);
    }
    // The last full row moves Down onto the final (shorter) row's last card.
    let rows = grid.len().div_ceil(6);
    state.focus = grid[(rows - 2) * 6 + 5].clone();
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(
        state.focus,
        grid[((rows - 2) * 6 + 11).min(grid.len() - 1)].clone()
    );
}

#[test]
fn the_shelf_scrolls_away_only_past_three_rows_and_returns_with_the_first() {
    let mut state = ready();
    let grid = state.grid();
    assert!(
        grid.len() > 3 * crate::state::APPS_COLUMNS,
        "The demo has more than three rows"
    );
    // The second and third rows fit beneath the compact shelf: nothing moves.
    for index in [7, 14] {
        state.focus = grid[index].clone();
        state.reveal_focus(&controls(&state));
        assert_eq!(state.scroll, 0., "Row of card {index}");
    }
    // The fourth row scrolls only as far as it needs, collapsing the shelf.
    state.focus = grid[19].clone();
    state.reveal_focus(&controls(&state));
    assert!(state.scroll > 0.);
    let row = controls(&state)
        .into_iter()
        .find(|control| control.target == grid[19])
        .unwrap()
        .rect;
    assert!(row.y + row.h <= 1080. - view::SCREEN_MARGIN + 0.01);
    assert!(row.y >= crate::state::GRID_TOP_LIMIT);
    state.focus = grid[1].clone();
    state.reveal_focus(&controls(&state));
    assert_eq!(state.scroll, 0.);
    // The greeting is drawn faded once the page has scrolled.
    state.scroll = 600.;
    let greeting = state.preferences.language.greeting(0, "");
    let faded = scene(&state).draw.into_iter().any(|draw| {
        matches!(draw, crate::renderer::Draw::Text(label, _, _, color, _) if label == greeting && color[3] < 0.1)
    });
    assert!(faded);
}

#[test]
fn eighteen_cards_fit_in_three_rows_beneath_a_compact_shelf_without_scrolling() {
    let mut state = State::new(Preferences::default());
    let mut inventory = demo_inventory();
    inventory.apps.truncate(18);
    state.set_inventory(inventory, None);
    let grid = state.grid();
    // Eighteen apps; Settings is an icon in the top bar.
    assert_eq!(grid.len(), 18);
    for target in &grid {
        state.focus = target.clone();
        state.reveal_focus(&controls(&state));
        assert_eq!(state.scroll, 0., "{target:?}");
    }
    let rendered = scene(&state);
    let cards: Vec<_> = rendered
        .controls
        .iter()
        .filter(|control| grid.contains(&control.target))
        .map(|control| control.rect)
        .collect();
    assert_eq!(cards.len(), 18);
    let first = cards.iter().map(|rect| rect.y).fold(f32::MAX, f32::min);
    let last = cards.iter().map(|rect| rect.y + rect.h).fold(0., f32::max);
    assert!(
        cards
            .iter()
            .all(|rect| rect.x >= 80. && rect.x + rect.w <= 1840.)
    );
    // The third row ends as far above the bottom as the clock's digits start below the top.
    assert_eq!(1080. - last, view::SCREEN_MARGIN, "Last row ends at {last}");
    assert_eq!(first, view::grid_y(&state));
    // A tighter row gap than the HIG's 100 px, still holding the focused caption.
    let gap = view::ROW_PITCH - view::CARD_HEIGHT;
    assert!((72. ..=80.).contains(&gap));
    assert!(gap >= view::CAPTION_SPACE);
    // The shelf sits directly beneath the clock line: date on it, greeting and weather below.
    // The test scene renders at the Unix epoch (midnight).
    let greeting = state.preferences.language.greeting(0, "");
    let greeting_rect = rendered
        .draw
        .iter()
        .find_map(|draw| match draw {
            crate::renderer::Draw::Text(label, rect, ..) if *label == greeting => Some(*rect),
            _ => None,
        })
        .unwrap();
    assert!(greeting_rect.y < view::HEADER.y + view::HEADER.h + 20.);
    assert!(greeting_rect.y + greeting_rect.h < first);
}

#[test]
fn hidden_apps_leave_the_grid_and_return_from_settings_hidden_apps() {
    use crate::state::Subpage;
    let mut state = ready();
    state.open(Page::Home);
    state.focus = Target::App("netflix".into());
    state.options();
    state.focus = Target::Option(1);
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert!(state.preferences.hidden.contains(&"netflix".into()));
    assert_eq!(state.focus, Target::App("youtube.leanback.v4".into()));
    assert!(!state.grid().contains(&Target::App("netflix".into())));
    // The notice points to where hidden apps live now.
    let notice = state.notice.clone().unwrap().0;
    assert!(
        notice.contains(state.preferences.language.t("hiddenApps")),
        "{notice}"
    );
    // Settings › General › Hidden apps lists it; OK shows it again.
    state.open(Page::Settings);
    state.focus = Target::Setting(11);
    assert!(
        controls(&state)
            .iter()
            .any(|c| c.target == Target::Setting(11))
    );
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.subpage, Some(Subpage::HiddenApps));
    assert_eq!(state.focus, Target::Choice(0));
    let rendered = scene(&state);
    assert!(
        rendered.draw.iter().any(
            |draw| matches!(draw, crate::renderer::Draw::Text(label, ..) if label == "Netflix")
        )
    );
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert!(state.preferences.hidden.is_empty());
    assert!(state.grid().contains(&Target::App("netflix".into())));
    // With nothing hidden the subpage shows its empty state, which holds focus.
    assert_eq!(state.subpage, Some(Subpage::HiddenApps));
    assert_eq!(state.focus, Target::Choice(0));
    let empty = scene(&state);
    let hint = state.preferences.language.t("hiddenHint");
    assert!(
        empty
            .draw
            .iter()
            .any(|draw| matches!(draw, crate::renderer::Draw::Text(label, ..) if label == hint))
    );
    assert!(empty.controls.iter().any(|c| c.target == Target::Choice(0)));
    assert!(
        state.activate().is_empty(),
        "The empty state does nothing on OK"
    );
    assert!(state.back().is_empty());
    assert_eq!(state.subpage, None);
    assert_eq!(state.focus, Target::Setting(11));
}

#[test]
fn hidden_apps_lists_only_installed_hidden_apps_by_title_and_keeps_focus_in_the_list() {
    use crate::state::Subpage;
    let mut state = ready();
    state.preferences.hidden = vec![
        "youtube.leanback.v4".into(),
        "netflix".into(),
        "uninstalled.app".into(),
    ];
    let titles: Vec<_> = state
        .hidden_apps()
        .iter()
        .map(|app| app.title.clone())
        .collect();
    assert_eq!(titles, ["Netflix", "YouTube"]);
    state.open(Page::Settings);
    state.focus = Target::Setting(11);
    state.activate();
    assert_eq!(state.subpage, Some(Subpage::HiddenApps));
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Choice(1));
    state.activate();
    // YouTube is shown again; focus stays on the remaining last row.
    assert!(
        !state
            .preferences
            .hidden
            .contains(&"youtube.leanback.v4".into())
    );
    assert_eq!(state.focus, Target::Choice(0));
    // The General row shows how many are hidden.
    state.back();
    let count = scene(&state)
        .draw
        .into_iter()
        .any(|draw| matches!(draw, crate::renderer::Draw::Text(label, ..) if label == "1"));
    assert!(count);
}

/// Long press on an app, then Move (the menu's first item): move mode for that app.
fn start_moving(state: &mut State, id: &str) {
    state.focus = Target::App(id.into());
    state.hold();
    assert!(matches!(&state.overlay, Some(Overlay::Options { id: open, .. }) if open == id));
    assert_eq!(state.focus, Target::Option(0), "Move is focused first");
    assert!(state.activate().is_empty());
    assert_eq!(state.moving.as_deref(), Some(id));
    assert!(state.overlay.is_none());
}

#[test]
fn reorder_persists_order_and_keeps_selected_identity() {
    let mut state = ready();
    start_moving(&mut state, "netflix");
    assert_eq!(
        state.direction(Direction::Right, &controls(&state)),
        vec![Effect::Save]
    );
    let order =
        |state: &State| -> Vec<String> { state.apps().iter().map(|app| app.id.clone()).collect() };
    assert_eq!(&order(&state)[..2], &["youtube.leanback.v4", "netflix"]);
    assert_eq!(
        &state.preferences.app_order.as_ref().unwrap()[..2],
        &["youtube.leanback.v4", "netflix"]
    );
    assert_eq!(state.focus, Target::App("netflix".into()));
    assert!(state.back().is_empty());
    assert!(state.moving.is_none());
    // Any app, not only the former favourites, can be moved the same way; OK confirms.
    let other = state.apps()[7].id.clone();
    start_moving(&mut state, &other);
    state.direction(Direction::Left, &controls(&state));
    assert_eq!(order(&state)[6], other);
    assert!(state.activate().is_empty());
    assert!(state.moving.is_none());
    assert!(
        state.overlay.is_none(),
        "OK in move mode confirms, never launches"
    );
}

#[test]
fn there_is_no_app_search_and_the_keyboard_stays_for_text_fields() {
    let mut state = ready();
    state.open(crate::state::Page::Home);
    let home = controls(&state);
    assert!(home.iter().all(|c| !matches!(c.target, Target::Key(_))));
    assert_eq!(
        home.iter()
            .filter(|c| c.target == Target::SettingsIcon)
            .count(),
        1
    );
    let search = state.preferences.language.t("search");
    assert!(
        !scene(&state)
            .draw
            .iter()
            .any(|draw| matches!(draw, crate::renderer::Draw::Text(label, ..) if label == search))
    );
    // The weather location still uses the modal keyboard.
    state.focus = Target::WeatherConfigure;
    state.activate();
    assert!(state.text_input_active());
    assert!(
        controls(&state)
            .iter()
            .all(|c| matches!(c.target, Target::Key(_) | Target::CloseDialog))
    );
    state.text("wenen");
    state.delete();
    assert_eq!(state.editing_text(), "wene");
}

#[test]
fn partial_refresh_preserves_valid_inputs_and_never_invents_tv_apps() {
    let mut state = ready();
    state.set_inventory(
        crate::model::Inventory::default(),
        Some("Inputs unavailable".into()),
    );
    assert!(state.inventory.apps.is_empty());
    assert_eq!(state.inventory.inputs.len(), 3);
    // Without TV apps the grid still opens on its Settings card.
    assert_eq!(state.first_content(), Target::SettingsIcon);
}

#[test]
fn inventory_filters_system_apps_duplicates_and_unsafe_identifiers() {
    let apps = normalize_apps(&json!([
        {"id":"netflix","title":" Netflix\n"}, {"id":"netflix","title":"Duplicate"},
        {"id":"bad;command","title":"Bad"}, {"id":"com.webos.app.hidden","title":"Internal"},
        {"id":"com.webos.app.livetv","visible":false,"title":"Live TV"},
        {"id":"com.luma.launcher.native","visible":true},
        {"id":"com.webos.app.hdmi1","visible":true},
        {"id":"hidden","visible":"false"}
    ]))
    .unwrap();
    assert_eq!(apps.len(), 2);
    assert_eq!(
        apps.iter().find(|a| a.id == "netflix").unwrap().title,
        "Netflix"
    );
    assert!(normalize_apps(&json!({"apps":[]})).is_err());
    let inputs = normalize_inputs(&json!([
        {"appId":"com.webos.app.hdmi1","id":"HDMI_1","port":1,"connected":true},
        {"appId":"com.webos.app.hdmi2","id":"HDMI_4"},
        {"appId":"com.webos.app.hdmi3"}, {"appId":"com.webos.app.hdmi3"},
        {"appId":"com.webos.app.hdmi4","label":"Console"}
    ]))
    .unwrap();
    assert_eq!(inputs.len(), 2);
    assert_eq!(inputs[1].connected, None);
}

#[test]
fn inventory_visibility_cases_match_the_launcher_filters() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../tests/fixtures/app-visibility.json")).unwrap();
    let inventory: Vec<_> = cases.iter().map(|case| case["app"].clone()).collect();
    let actual: std::collections::BTreeSet<_> = normalize_apps(&json!(inventory))
        .unwrap()
        .into_iter()
        .map(|app| app.id)
        .collect();
    let expected: std::collections::BTreeSet<_> = cases
        .iter()
        .filter(|case| case["include"] == true)
        .map(|case| case["app"]["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn asset_loading_and_unknown_app_cards_do_not_require_collected_artwork() {
    let root = std::env::temp_dir().join(format!("luma-assets-no-dumps-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("version.txt"), "test").unwrap();
    std::fs::write(root.join("font-metrics.json"), "{}").unwrap();
    for webos in [false, true] {
        let assets = view::Assets::load(root.clone(), webos).unwrap();
        let mut state = State::new(Preferences::default());
        state.set_inventory(
            crate::model::Inventory {
                apps: vec![App::new("com.example.regional", "Regional App")],
                inputs: vec![],
            },
            None,
        );
        state.open(crate::state::Page::Home);
        let scene = view::scene(&state, &assets, time::OffsetDateTime::UNIX_EPOCH);
        assert!(scene.draw.iter().any(|draw| matches!(draw,
            crate::renderer::Draw::TintedImage(path, ..) if path.ends_with("ui-icons/generic.lvec"))));
        assert!(scene.draw.iter().any(|draw| matches!(draw,
            crate::renderer::Draw::Text(value, ..) if value == "Regional App")));
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn auto_input_visibility_requires_a_positive_connection_report() {
    let mut preferences = Preferences::default();
    let mut input = Input {
        id: "com.webos.app.hdmi1".into(),
        title: "Console".into(),
        port: "HDMI 1".into(),
        connected: None,
        signal: None,
    };
    assert!(!preferences.input_visible(&input));
    input.connected = Some(true);
    assert!(preferences.input_visible(&input));
    preferences
        .input_modes
        .insert(input.id.clone(), InputMode::Off);
    assert!(!preferences.input_visible(&input));
    preferences
        .input_modes
        .insert(input.id.clone(), InputMode::On);
    input.connected = Some(false);
    assert!(preferences.input_visible(&input));
}

#[test]
fn preferences_round_trip_and_corrupt_data_is_not_silently_overwritten() {
    let directory = std::env::temp_dir().join(format!("luma-native-test-{}", std::process::id()));
    let path = directory.join("preferences.json");
    let mut preferences = Preferences {
        favorites: Some(vec![]),
        background: "forest".into(),
        ..Preferences::default()
    };
    preferences
        .input_modes
        .insert("com.webos.app.hdmi1".into(), InputMode::Off);
    storage::save(&path, &preferences).unwrap();
    let loaded = storage::load(&path).unwrap();
    assert_eq!(loaded.favorites, Some(vec![]));
    assert_eq!(loaded.background, "forest");
    assert_eq!(loaded.input_modes["com.webos.app.hdmi1"], InputMode::Off);
    std::fs::write(&path, "broken").unwrap();
    assert!(storage::load(&path).is_err());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn gallery_and_grid_navigation_scroll_to_keep_focus_visible() {
    let mut state = ready();
    state.set_wallpapers(vec![crate::wallpapers::Wallpaper {
        id: "custom-00000000000000000000000000000001".into(),
        directory: PathBuf::from("/preview/wallpapers/custom-00000000000000000000000000000001"),
        has_original: true,
        blurred: [false; 2],
    }]);
    state.open(crate::state::Page::Settings);
    state.section = 1;
    state.focus = Target::Background(0);
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Background(3));
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Background(6));
    assert_eq!(state.scroll, 0., "the first three gallery rows fit");
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Background(9));
    assert!(state.scroll > 0., "the fourth gallery row must scroll");
    let control = controls(&state)
        .into_iter()
        .find(|c| c.target == state.focus)
        .unwrap();
    let viewport = state.backgrounds_viewport();
    assert!(control.rect.y >= viewport.y);
    assert!(control.rect.y + control.rect.h <= viewport.y + viewport.h);
    state.direction(Direction::Left, &controls(&state));
    assert_eq!(state.focus, Target::Sidebar(1));
    state.open(crate::state::Page::Home);
    let last = state.apps().last().unwrap().id.clone();
    state.focus(Target::App(last), &controls(&state));
    assert!(
        state.scroll > 0.,
        "library focus includes its wrapped caption"
    );
}

#[test]
fn cancelling_options_preserves_identity_and_does_not_save() {
    let mut state = ready();
    state.focus = Target::App("netflix".into());
    state.options();
    assert!(matches!(state.overlay, Some(Overlay::Options { .. })));
    state.focus = Target::CloseDialog;
    assert!(state.activate().is_empty());
    assert_eq!(state.focus, Target::App("netflix".into()));
    assert!(state.overlay.is_none());
}

#[test]
fn pointer_focus_does_not_scroll_the_clicked_control() {
    let mut state = ready();
    state.open(crate::state::Page::Home);
    state.scroll = 150.;
    state.pointer_focus(Target::App("netflix".into()));
    assert_eq!(state.scroll, 150.);
    // Remote focus on the first row brings the shelf back.
    state.focus(Target::App("netflix".into()), &controls(&state));
    assert_eq!(state.scroll, 0.);
}

#[test]
fn gallery_right_crosses_rows_in_gallery_order() {
    let mut state = ready();
    state.open(crate::state::Page::Settings);
    state.section = 1;
    state.focus = Target::Background(2);
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.focus, Target::Background(3));
}

#[test]
fn reordering_skips_hidden_and_unavailable_favorite_slots() {
    let mut state = ready();
    state.preferences.app_order = Some(vec![
        "netflix".into(),
        "missing".into(),
        "youtube.leanback.v4".into(),
        "amazon".into(),
    ]);
    state.preferences.hidden.push("youtube.leanback.v4".into());
    start_moving(&mut state, "netflix");
    state.direction(Direction::Right, &controls(&state));
    // Hidden and missing apps keep their places in the order.
    assert_eq!(
        &state.preferences.app_order.unwrap()[..4],
        &["amazon", "missing", "youtube.leanback.v4", "netflix"]
    );
    assert_eq!(state.focus, Target::App("netflix".into()));
}

#[test]
fn greeting_subpage_opens_the_keyboard_and_back_restores_its_parent() {
    let mut state = ready();
    state.open(crate::state::Page::Settings);
    state.focus = Target::Widget(0);
    assert!(
        state
            .direction(Direction::Right, &controls(&state))
            .is_empty()
    );
    assert_eq!(state.subpage, Some(crate::state::Subpage::Greeting));
    assert_eq!(state.focus, Target::GreetingToggle);
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::GreetingName);
    assert!(!state.text_input_active());
    assert!(state.activate().is_empty());
    assert!(state.text_input_active());
    state.text("Ana");
    assert_eq!(state.editing_text(), "Ana");
    assert!(state.preferences.greeting_name.is_empty());
    assert_eq!(state.finish_text_input(), vec![Effect::Save]);
    assert_eq!(state.focus, Target::GreetingName);
    assert!(state.finish_text_input().is_empty());
    assert_eq!(
        crate::i18n::Language::En.greeting(9, &state.preferences.greeting_name),
        "Good morning, Ana."
    );
    assert_eq!(state.subpage, Some(crate::state::Subpage::Greeting));
    assert!(state.back().is_empty());
    assert!(state.subpage.is_none());
    assert_eq!(state.focus, Target::Widget(0));
}

#[test]
fn greeting_name_replacement_commits_on_every_keyboard_completion_route() {
    for route in 0..3 {
        let mut state = ready();
        state.preferences.greeting_name = "Ana".into();
        state.open(crate::state::Page::Settings);
        state.focus = Target::Widget(0);
        state.activate();
        state.direction(Direction::Down, &controls(&state));
        state.activate();
        assert!(state.editing_text().is_empty());
        state.text("  BobX ");
        state.delete_surrounding_text(-3, 3);
        assert_eq!(state.editing_text(), "  Bo");
        state.text("b  ");
        assert_eq!(state.preferences.greeting_name, "Ana");
        let effects = match route {
            0 => {
                state.overlay = Some(Overlay::SystemKeyboard);
                state.finish_text_input()
            }
            1 => state.back(),
            _ => {
                state.focus = Target::Key(crate::state::KEYS.len() - 1);
                state.activate()
            }
        };
        assert_eq!(effects, vec![Effect::Save]);
        assert_eq!(state.preferences.greeting_name, "Bob");
        assert_eq!(state.focus, Target::GreetingName);
        assert!(state.finish_text_input().is_empty());
        state.activate();
        state.text("   ");
        assert!(state.back().is_empty());
        assert_eq!(state.preferences.greeting_name, "Bob");
    }
}

#[test]
fn remote_can_clear_a_saved_greeting_name_without_moving_the_controls() {
    let mut state = ready();
    state.preferences.greeting_name = "Ana".into();
    state.open(crate::state::Page::Settings);
    state.focus = Target::Widget(0);
    state.activate();
    let before: Vec<_> = controls(&state)
        .iter()
        .map(|c| (c.target.clone(), c.rect.y))
        .collect();
    state.direction(Direction::Down, &controls(&state));
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::GreetingNameClear);
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert!(state.preferences.greeting_name.is_empty());
    assert_eq!(state.focus, Target::GreetingName);
    let after: Vec<_> = controls(&state)
        .iter()
        .map(|c| (c.target.clone(), c.rect.y))
        .collect();
    assert_eq!(before, after);
    state.direction(Direction::Left, &controls(&state));
    assert_eq!(state.focus, Target::Widget(0));
    assert!(state.subpage.is_none());
}

#[test]
fn app_names_are_one_toggle_for_the_focused_grid_card() {
    // Legacy preferences (independent Home/Apps toggles) keep the Apps choice.
    let old: Preferences =
        serde_json::from_value(json!({"show_names": true, "show_home_names": true})).unwrap();
    assert!(old.show_app_names);
    let mut state = ready();
    assert!(state.preferences.show_app_names);
    state.open(Page::Settings);
    state.focus = Target::Setting(0);
    // Right does not toggle a value row; OK does, without opening a subpage.
    assert!(
        state
            .direction(Direction::Right, &controls(&state))
            .is_empty()
    );
    assert!(state.preferences.show_app_names);
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert!(!state.preferences.show_app_names);
    assert!(state.subpage.is_none());
    assert_eq!(state.focus, Target::Setting(0));
    let restored: Preferences =
        serde_json::from_value(serde_json::to_value(&state.preferences).unwrap()).unwrap();
    assert!(!restored.show_app_names);
    for names in [true, false] {
        state.preferences.show_app_names = names;
        state.open(Page::Home);
        state.focus = Target::App("youtube.leanback.v4".into());
        let scene = scene(&state);
        // tvOS shows a title only beneath the focused card.
        let has = |title: &str| {
            scene.draw.iter().any(
                |draw| matches!(draw, crate::renderer::Draw::Text(value, ..) if value == title),
            )
        };
        assert_eq!(has("YouTube"), names);
        assert!(!has("Netflix"));
        let card = scene
            .controls
            .iter()
            .find(|c| c.target == Target::App("youtube.leanback.v4".into()))
            .unwrap();
        assert_eq!(
            card.rect.h,
            view::CARD_HEIGHT,
            "Captions never reserve space or extend the hit region"
        );
    }
}

#[test]
fn input_name_actions_only_highlight_the_focused_row_and_keep_the_saved_mode() {
    let mut state = ready();
    state
        .preferences
        .set_input_name("com.webos.app.hdmi1", "Console");
    state.open(crate::state::Page::Settings);
    state.focus = Target::Widget(3);
    state.activate();
    state.focus = Target::InputMode("com.webos.app.hdmi1".into());
    state.activate();
    for target in [Target::InputRename, Target::InputNameReset] {
        state.pointer_focus(target.clone());
        let scene = scene(&state);
        let highlighted = highlighted_rows(&scene);
        let focused = scene.controls.iter().find(|c| c.target == target).unwrap();
        assert_eq!(highlighted, vec![focused.rect.center().1]);
        assert!(scene.draw.iter().any(|draw| matches!(draw, crate::renderer::Draw::TintedImage(path, ..) if path.ends_with("check.lvec"))));
        assert_eq!(
            state
                .preferences
                .input_modes
                .get("com.webos.app.hdmi1")
                .copied()
                .unwrap_or_default(),
            InputMode::Auto
        );
        assert!(scene.draw.iter().any(
            |draw| matches!(draw, crate::renderer::Draw::Text(value, ..) if value == "Clear name")
        ));
    }
}

#[test]
fn clock_choices_only_highlight_focus_while_preserving_saved_checkmarks() {
    let mut state = ready();
    state.preferences.show_seconds = true;
    state.open(crate::state::Page::Settings);
    state.focus = Target::Setting(1);
    state.activate();
    state.activate();
    for index in 0..2 {
        state.pointer_focus(Target::Choice(index));
        let scene = scene(&state);
        let highlighted = highlighted_rows(&scene);
        let focused = scene
            .controls
            .iter()
            .find(|c| c.target == Target::Choice(index))
            .unwrap();
        assert_eq!(highlighted, vec![focused.rect.center().1]);
        let checks = scene.draw.iter().filter(|draw| matches!(draw, crate::renderer::Draw::TintedImage(path, ..) if path.ends_with("check.lvec"))).count();
        assert_eq!(checks, 1);
        assert!(!state.preferences.uses_24_hour());
        assert!(state.preferences.show_seconds);
    }
}

#[test]
fn clock_format_subpage_saves_and_restores_its_parent_row() {
    let mut state = ready();
    state.open(crate::state::Page::Settings);
    state.focus = Target::Setting(1);
    assert!(
        state
            .direction(Direction::Right, &controls(&state))
            .is_empty()
    );
    assert_eq!(state.subpage, Some(crate::state::Subpage::Clock));
    assert_eq!(state.focus, Target::ClockFormat);
    assert!(
        state
            .direction(Direction::Right, &controls(&state))
            .is_empty()
    );
    assert_eq!(state.subpage, Some(crate::state::Subpage::ClockFormat));
    assert_eq!(state.focus, Target::Choice(0));
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert!(state.preferences.uses_24_hour());
    assert!(!state.preferences.show_seconds);
    assert_eq!(state.subpage, Some(crate::state::Subpage::Clock));
    assert_eq!(state.focus, Target::ClockFormat);
    state.direction(Direction::Left, &controls(&state));
    assert!(state.subpage.is_none());
    assert_eq!(state.focus, Target::Setting(1));
    state.activate();
    state.activate();
    state.direction(Direction::Left, &controls(&state));
    assert_eq!(state.subpage, Some(crate::state::Subpage::Clock));
    assert_eq!(state.focus, Target::ClockFormat);
    state.back();
    assert_eq!(state.focus, Target::Setting(1));
    assert!(state.subpage.is_none());
}

#[test]
fn time_format_selects_seconds_without_changing_the_hour_format() {
    let mut state = ready();
    state.preferences.hour_24 = true;
    state.open(crate::state::Page::Settings);
    state.focus = Target::Setting(1);
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.subpage, Some(crate::state::Subpage::Clock));
    assert!(!state.preferences.show_seconds);
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::ClockTimeFormat);
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.subpage, Some(crate::state::Subpage::TimeFormat));
    assert_eq!(state.focus, Target::Choice(0));
    assert!(
        !controls(&state)
            .iter()
            .any(|c| c.target == Target::Choice(2))
    );
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Choice(1));
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert!(state.preferences.show_seconds);
    assert!(state.preferences.uses_24_hour());
    assert_eq!(state.subpage, Some(crate::state::Subpage::Clock));
    assert_eq!(state.focus, Target::ClockTimeFormat);
    state.activate();
    assert_eq!(state.focus, Target::Choice(1));
    state.back();
    assert_eq!(state.subpage, Some(crate::state::Subpage::Clock));
    assert_eq!(state.focus, Target::ClockTimeFormat);
    state.activate();
    state.direction(Direction::Up, &controls(&state));
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert!(!state.preferences.show_seconds);
    assert!(state.preferences.uses_24_hour());
    state.back();
    assert!(state.subpage.is_none());
    assert_eq!(state.focus, Target::Setting(1));
}

#[test]
fn system_version_is_inside_the_update_status_card_without_a_separate_row() {
    let mut state = ready();
    state.open(crate::state::Page::Settings);
    state.section = crate::state::SYSTEM_SECTION;
    let system = scene(&state);
    // One glass status card holds the update state and the installed version.
    assert!(system.draw.iter().any(|draw| matches!(
        draw,
        crate::renderer::Draw::Backdrop(_, rect, ..)
            if rect.x == view::SETTINGS_X && rect.y == view::SETTINGS_TOP && rect.w == view::SETTINGS_W && rect.h == 256.
    )));
    assert!(system.draw.iter().any(|draw| matches!(
        draw,
        crate::renderer::Draw::Text(label, _, style, color, _)
            if label == "Current version: test" && style.size == 29. && style.weight == 400 && color[3] < 1.
    )));
    assert_eq!(
        state.hit_test(&system.controls, 900., view::SETTINGS_TOP + 60.),
        None
    );
    state.focus = Target::Sidebar(crate::state::SYSTEM_SECTION);
    state.direction(Direction::Right, &system.controls);
    assert_eq!(state.focus, Target::CheckUpdates);
}

#[test]
fn system_category_owns_refresh_and_quit_and_restores_sidebar_focus() {
    let mut state = ready();
    state.open(crate::state::Page::Settings);
    assert!(
        !controls(&state)
            .iter()
            .any(|c| matches!(c.target, Target::Setting(3 | 5)))
    );
    assert!(
        controls(&state)
            .iter()
            .any(|c| c.target == Target::Sidebar(crate::state::SYSTEM_SECTION))
    );
    state.focus = Target::Sidebar(crate::state::SYSTEM_SECTION - 1);
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Sidebar(crate::state::SYSTEM_SECTION));
    assert_eq!(state.section, crate::state::SYSTEM_SECTION);
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.focus, Target::CheckUpdates);
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Setting(3));
    assert_eq!(state.activate(), vec![Effect::Reload]);
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Setting(5));
    assert_eq!(state.activate(), vec![Effect::Quit]);
    state.back();
    assert_eq!(state.focus, Target::Sidebar(crate::state::SYSTEM_SECTION));
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Sidebar(0));
    assert_eq!(state.section, 0);
}

#[test]
fn the_removed_text_rendering_choice_loads_and_leaves_no_system_row() {
    // Text has one path now (the CPU glyph atlas); a saved choice is ignored and dropped.
    for value in ["msaa", "analytic"] {
        let preferences: Preferences =
            serde_json::from_value(json!({"text_rendering": value, "greeting_name": "Ana"}))
                .unwrap();
        assert_eq!(preferences.greeting_name, "Ana");
        let saved = serde_json::to_value(&preferences).unwrap();
        assert!(!saved.as_object().unwrap().contains_key("text_rendering"));
    }
    let mut state = ready();
    state.open(crate::state::Page::Settings);
    state.section = crate::state::SYSTEM_SECTION;
    let rows: Vec<_> = controls(&state)
        .into_iter()
        .filter_map(|control| match control.target {
            Target::Setting(index) => Some(index),
            _ => None,
        })
        .collect();
    assert!(!rows.contains(&7), "{rows:?}");
    assert!(rows.contains(&3) && rows.contains(&5), "{rows:?}");
}

#[test]
fn input_visibility_has_explicit_choices_and_returns_to_the_same_input() {
    let mut state = ready();
    state.open(crate::state::Page::Settings);
    state.focus = Target::Widget(3);
    state.activate();
    let id = "com.webos.app.hdmi1".to_string();
    state.focus = Target::InputMode(id.clone());
    state.activate();
    assert_eq!(state.focus, Target::Choice(2));
    state.direction(Direction::Up, &controls(&state));
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert_eq!(state.preferences.input_modes[&id], InputMode::Off);
    assert_eq!(state.focus, Target::InputMode(id.clone()));
    assert!(!state.preferences.input_visible(&state.inventory.inputs[0]));
    state.activate();
    state.back();
    assert_eq!(state.focus, Target::InputMode(id));
    assert_eq!(state.subpage, Some(crate::state::Subpage::Inputs));
    state.back();
    assert_eq!(state.focus, Target::Widget(3));
}

#[test]
fn background_sidebar_right_focuses_upload_and_ok_restores_the_applied_background() {
    let mut state = ready();
    state.open(crate::state::Page::Settings);
    state.preferences.background = "forest".into();
    state.focus = Target::Sidebar(1);
    state.section = 1;
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.focus, Target::UploadWallpaper);
    assert_eq!(state.preferences.background, "forest");
    assert!(state.overlay.is_none());
    // Upload is the only action: Right stays on it.
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.focus, Target::UploadWallpaper);
    state.direction(Direction::Left, &controls(&state));
    assert_eq!(state.focus, Target::Sidebar(1));
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.focus, Target::UploadWallpaper);
    state.back();
    state.activate();
    assert_eq!(state.focus, Target::Background(6));
}

#[test]
fn a_notice_does_not_escape_modal_focus_and_closing_restores_the_app() {
    let mut state = ready();
    state.open(crate::state::Page::Home);
    state.notify("Saved");
    state.focus = Target::App("netflix".into());
    state.options();
    assert!(
        controls(&state)
            .iter()
            .all(|c| matches!(c.target, Target::OpenApp | Target::Option(_)))
    );
    state.focus = Target::Option(1);
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Option(2));
    state.back();
    assert_eq!(state.focus, Target::App("netflix".into()));
}

#[test]
fn delete_confirmation_defaults_to_cancel_and_back_restores_side_popup() {
    let mut state = ready();
    state.open(crate::state::Page::Home);
    state.focus = Target::App("netflix".into());
    let card = controls(&state)
        .into_iter()
        .find(|c| c.target == state.focus)
        .unwrap()
        .rect;
    let parent_draw_count = scene(&state).draw.len();
    state.options();
    let popup_scene = scene(&state);
    let popup = popup_scene.modal.unwrap();
    assert!(
        popup_scene
            .controls
            .iter()
            .all(|control| matches!(control.target, Target::OpenApp | Target::Option(_)))
    );
    assert!(!popup_scene.draw[parent_draw_count..].iter().any(|draw| matches!(draw,
        crate::renderer::Draw::Text(value, rect, ..) if popup.contains(rect.x, rect.y)
            && ["Netflix", "Streaming", "Select", "Close", "OK", "Back"].contains(&value.as_str()))));
    let action_icons: Vec<_> = popup_scene.draw[parent_draw_count..]
        .iter()
        .filter_map(|draw| match draw {
            crate::renderer::Draw::Image(_, rect, ..)
            | crate::renderer::Draw::TintedImage(_, rect, ..)
                if popup.contains(rect.x, rect.y) =>
            {
                Some(rect)
            }
            _ => None,
        })
        .collect();
    assert_eq!(action_icons.len(), 4);
    assert!(
        action_icons
            .iter()
            .all(|rect| rect.x > popup.x + popup.w - 80.)
    );
    assert!(popup.x >= card.x + card.w || popup.x + popup.w <= card.x);
    state.focus = Target::Option(2);
    let focused_scene = scene(&state);
    let delete_rect = focused_scene
        .controls
        .iter()
        .find(|c| c.target == Target::Option(2))
        .unwrap()
        .rect;
    // A focused destructive action is the white focused surface with a red label.
    assert!(focused_scene.draw.iter().any(|draw| matches!(draw,
        crate::renderer::Draw::RoundedGradient(rect, _, top, _) if rect.contains(delete_rect.center().0, delete_rect.center().1)
            && rect.w >= delete_rect.w && *top == [1., 1., 1., 1.])));
    assert!(focused_scene.draw.iter().any(|draw| matches!(draw,
        crate::renderer::Draw::Text(label, rect, _, color, _) if label == "Delete app" && delete_rect.contains(rect.x, rect.y)
            && color[0] > 0.8 && color[1] < 0.1)));
    assert!(state.activate().is_empty());
    assert!(matches!(state.overlay, Some(Overlay::Delete { .. })));
    assert_eq!(state.focus, Target::CancelDelete);
    let confirmation = scene(&state).modal.unwrap();
    assert_eq!(confirmation.w, 880.);
    assert!(confirmation.h < confirmation.w);
    assert!(
        controls(&state)
            .iter()
            .all(|c| matches!(c.target, Target::ConfirmDelete | Target::CancelDelete))
    );
    assert!(state.activate().is_empty());
    assert!(matches!(state.overlay, Some(Overlay::Options { .. })));
    assert_eq!(state.focus, Target::Option(2));
    state.activate();
    state.back();
    assert_eq!(state.focus, Target::Option(2));
    state.back();
    assert_eq!(state.focus, Target::App("netflix".into()));
}

#[test]
fn opening_side_popup_preserves_card_artwork_geometry_with_or_without_captions() {
    {
        for names in [false, true] {
            let mut state = ready();
            state.preferences.show_app_names = names;
            state.open(Page::Home);
            state.focus = Target::App("netflix".into());
            let before = scene(&state);
            let logo_rect = |draws: &[crate::renderer::Draw]| {
                draws
                    .iter()
                    .filter_map(|draw| match draw {
                        crate::renderer::Draw::Image(path, rect, ..)
                        | crate::renderer::Draw::TintedImage(path, rect, ..)
                            if path.contains("netflix") =>
                        {
                            Some([rect.x, rect.y, rect.w, rect.h])
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            let original = logo_rect(&before.draw);
            assert!(!original.is_empty());
            state.options();
            let lifted = logo_rect(&scene(&state).draw);
            assert_eq!(lifted.len(), original.len() * 2);
            // The parent copy rests under the dimmed stage; the popup's copy keeps the
            // focused card's lifted artwork bounds exactly.
            for (actual, expected) in lifted[original.len()..].iter().zip(original.iter()) {
                for (a, b) in actual.iter().zip(expected) {
                    assert!(
                        (a - b).abs() < 0.001,
                        "Artwork bounds changed: {actual:?} vs {expected:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn protected_apps_cannot_be_deleted_through_ui_or_worker() {
    for id in [
        crate::platform::APP_ID,
        crate::platform::HOME_ID,
        "com.webos.app.livetv",
        "com.webos.app.browser",
        "com.webos.app.settings",
        "com.webos.service.applicationmanager",
        "com.palm.app.settings",
        "com.palm.app.input",
        "com.lge.app.store",
        "com.lge.service.example",
        "com.luma.launcher",
        "com.luma.native",
        "com.luma.launcher.native",
        "org.webosbrew.hbchannel",
        "org.webosbrew.hbchannel.service",
        "custom.lamp",
        "invalid';command",
    ] {
        assert!(crate::platform::uninstall_request(id).is_err());
        let mut state = ready();
        state.inventory.apps.push(App::new(id, "Renamed app"));
        state.open(crate::state::Page::Home);
        state.focus = Target::App(id.into());
        state.options();
        assert!(
            !controls(&state)
                .iter()
                .any(|c| c.target == Target::Option(2))
        );
        state.focus = Target::Option(2);
        assert!(state.activate().is_empty());
        assert!(matches!(state.overlay, Some(Overlay::Options { .. })));
        state.overlay = Some(Overlay::Delete {
            id: id.into(),
            restore: Target::App(id.into()),
        });
        state.focus = Target::ConfirmDelete;
        assert!(state.activate().is_empty());
    }
    assert_eq!(
        crate::platform::uninstall_request("netflix").unwrap(),
        (
            "luna://com.webos.appInstallService/remove",
            json!({"id": "netflix", "subscribe": true})
        )
    );
    for id in [
        "com.example.app",
        "com.webosbrew.app",
        "com.palmtree.app",
        "com.lgeneric.app",
    ] {
        assert!(
            crate::platform::can_uninstall(id),
            "Unexpectedly protected: {id}"
        );
    }
}

#[test]
fn deletion_is_single_flight_and_only_success_cleans_up_preferences() {
    let mut state = ready();
    state.open(crate::state::Page::Home);
    state.preferences.favorites = Some(vec!["netflix".into(), "youtube.leanback.v4".into()]);
    state.preferences.hidden.push("netflix".into());
    state.focus = Target::App("netflix".into());
    state.options();
    state.focus = Target::Option(2);
    state.activate();
    state.focus = Target::ConfirmDelete;
    let cancel_before = controls(&state)
        .into_iter()
        .find(|c| c.target == Target::CancelDelete)
        .unwrap()
        .rect;
    assert_eq!(state.activate(), vec![Effect::Uninstall("netflix".into())]);
    assert_eq!(state.focus, Target::CancelDelete);
    let cancel_after = controls(&state)
        .into_iter()
        .find(|c| c.target == Target::CancelDelete)
        .unwrap()
        .rect;
    assert_eq!(
        [
            cancel_before.x,
            cancel_before.y,
            cancel_before.w,
            cancel_before.h
        ],
        [
            cancel_after.x,
            cancel_after.y,
            cancel_after.w,
            cancel_after.h
        ]
    );
    assert!(
        !controls(&state)
            .iter()
            .any(|c| c.target == Target::ConfirmDelete)
    );
    state.focus = Target::ConfirmDelete;
    assert!(state.activate().is_empty());
    assert!(state.uninstall_response("other".into(), Ok(())).is_empty());
    assert_eq!(state.uninstalling.as_deref(), Some("netflix"));
    assert_eq!(
        state.uninstall_response("netflix".into(), Err("denied".into())),
        vec![Effect::Refresh]
    );
    assert!(state.inventory.apps.iter().any(|a| a.id == "netflix"));
    assert!(state.preferences.hidden.contains(&"netflix".into()));
    assert_eq!(state.preferences.favorites.as_ref().unwrap().len(), 2);
    assert_eq!(state.focus, state.first_content());
    assert!(state.overlay.is_none());
    state.focus = Target::App("netflix".into());
    state.options();
    state.focus = Target::Option(2);
    state.activate();
    state.focus = Target::ConfirmDelete;
    state.activate();
    assert_eq!(
        state.uninstall_response("netflix".into(), Ok(())),
        vec![Effect::Save, Effect::Refresh]
    );
    assert!(!state.inventory.apps.iter().any(|a| a.id == "netflix"));
    assert!(state.preferences.hidden.is_empty());
    assert_eq!(
        state.preferences.favorites,
        Some(vec!["youtube.leanback.v4".into()])
    );
    assert!(state.overlay.is_none());
    assert_eq!(state.focus, Target::App("youtube.leanback.v4".into()));
}

#[test]
fn deletion_completion_does_not_steal_focus_after_dismissal() {
    let mut state = ready();
    state.uninstalling = Some("netflix".into());
    state.open(crate::state::Page::Settings);
    state.uninstall_response("netflix".into(), Ok(()));
    assert_eq!(state.page, Page::Settings);
    assert_eq!(state.focus, Target::Sidebar(0));
    assert!(state.overlay.is_none());
}

#[test]
fn app_activation_launches_and_options_open_with_move_first() {
    let mut state = ready();
    state.open(Page::Home);
    // tvOS: OK on a grid card launches it.
    state.focus = Target::App("netflix".into());
    assert_eq!(state.activate(), vec![Effect::Launch("netflix".into())]);
    assert!(state.overlay.is_none());
    // Menu (or a long press on any app) opens options with Move first and focused; Open is
    // just below it.
    state.options();
    assert!(matches!(state.overlay, Some(Overlay::Options { .. })));
    assert_eq!(state.focus, Target::Option(0));
    let menu = controls(&state);
    let row = |target: Target| menu.iter().find(|c| c.target == target).unwrap().rect.y;
    assert!(row(Target::Option(0)) < row(Target::OpenApp));
    assert!(row(Target::OpenApp) < row(Target::Option(1)));
    state.focus = Target::OpenApp;
    assert_eq!(state.activate(), vec![Effect::Launch("netflix".into())]);
    assert!(state.overlay.is_none());
    assert_eq!(state.focus, Target::App("netflix".into()));
}

#[test]
fn uncertain_deletion_uses_complete_inventory_instead_of_visible_app_filtering() {
    use crate::platform::inventory_confirms_removal;
    assert_eq!(inventory_confirms_removal(&json!([]), "netflix"), Ok(true));
    assert_eq!(
        inventory_confirms_removal(&json!([{"id": "youtube.leanback.v4"}]), "netflix"),
        Ok(true)
    );
    assert_eq!(
        inventory_confirms_removal(&json!([{"id": "netflix", "visible": false}]), "netflix"),
        Ok(false)
    );
    assert!(inventory_confirms_removal(&json!({}), "netflix").is_err());
    assert!(inventory_confirms_removal(&json!([{}]), "netflix").is_err());
}

#[test]
fn failed_deletion_refreshes_inventory_and_closes_stale_confirmation() {
    let mut state = ready();
    state.open(crate::state::Page::Home);
    state.focus = Target::App("netflix".into());
    state.options();
    state.focus = Target::Option(2);
    state.activate();
    state.focus = Target::ConfirmDelete;
    state.activate();
    assert_eq!(
        state.uninstall_response("netflix".into(), Err("timeout".into())),
        vec![Effect::Refresh]
    );
    assert!(state.overlay.is_none());
    assert_eq!(state.focus, Target::App("netflix".into()));
    let mut inventory = demo_inventory();
    inventory.apps.retain(|app| app.id != "netflix");
    state.set_inventory(inventory, None);
    assert!(!state.apps().iter().any(|app| app.id == "netflix"));
    assert_ne!(state.focus, Target::App("netflix".into()));
}

#[test]
fn long_captions_stay_on_one_line_inside_the_row_gap() {
    let mut state = ready();
    state.open(crate::state::Page::Home);
    let long = "A very long installed application title that needs more than two lines to remain fully readable";
    state
        .inventory
        .apps
        .iter_mut()
        .find(|app| app.id == "netflix")
        .unwrap()
        .title = long.into();
    state.focus = Target::App("netflix".into());
    let scene = scene(&state);
    let targets = &scene.controls;
    let title = targets
        .iter()
        .find(|c| c.target == Target::App("netflix".into()))
        .unwrap();
    let next_id = state.apps()[6].id.clone();
    let next = targets
        .iter()
        .find(|c| c.target == Target::App(next_id.clone()))
        .unwrap();
    // Rows keep the HIG pitch; the focused caption is one line, ending with an ellipsis.
    assert_eq!(title.rect.h, view::CARD_HEIGHT);
    assert_eq!(next.rect.y - title.rect.y, view::ROW_PITCH);
    let caption = scene
        .draw
        .iter()
        .find_map(|draw| match draw {
            crate::renderer::Draw::Text(label, rect, ..) if label.starts_with("A very long") => {
                Some((label.clone(), *rect))
            }
            _ => None,
        })
        .unwrap();
    assert!(caption.0.ends_with('…'));
    assert!(caption.1.y + caption.1.h < next.rect.y);
}

#[test]
fn showing_a_hidden_app_restores_it_to_its_place_and_leaves_hidden_empty() {
    let mut state = ready();
    state.open(crate::state::Page::Home);
    let place = state
        .apps()
        .iter()
        .position(|app| app.id == "org.jellyfin.webos")
        .unwrap();
    state.preferences.hidden.push("org.jellyfin.webos".into());
    assert!(
        !state
            .apps()
            .iter()
            .any(|app| app.id == "org.jellyfin.webos")
    );
    state.focus = Target::App("org.jellyfin.webos".into());
    state.options();
    // The hidden app's menu offers Show in place of Hide.
    state.focus = Target::Option(1);
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert!(state.preferences.hidden.is_empty());
    // It comes back to the slot it had in the user's order, not to the end.
    assert_eq!(state.apps()[place].id, "org.jellyfin.webos");
    assert_eq!(state.focus, Target::App("org.jellyfin.webos".into()));
}

#[test]
fn weather_location_keyboard_back_and_sidebar_routes_restore_focus() {
    let mut state = ready();
    state.open(crate::state::Page::Settings);
    state.focus = Target::Sidebar(crate::state::WIDGETS_SECTION);
    state.section = crate::state::WIDGETS_SECTION;
    state.activate();
    assert_eq!(state.focus, Target::Widget(4));
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Widget(0));
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Widget(1));
    state.activate();
    assert_eq!(state.focus, Target::WeatherToggle);
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::WeatherLocation);
    state.activate();
    assert_eq!(state.focus, Target::WeatherQuery);
    state.activate();
    state.text("Vienna");
    state.back();
    assert_eq!(state.focus, Target::WeatherSearch);
    assert_eq!(state.weather.query, "Vienna");
    assert_eq!(state.subpage, Some(crate::state::Subpage::WeatherLocation));
    state.direction(Direction::Left, &controls(&state));
    assert_eq!(state.focus, Target::WeatherLocation);
    assert_eq!(state.subpage, Some(crate::state::Subpage::Weather));
    state.back();
    assert_eq!(state.focus, Target::Widget(1));
    assert!(state.subpage.is_none());
    state.back();
    assert_eq!(state.focus, Target::Sidebar(crate::state::WIDGETS_SECTION));
}

#[test]
fn weather_units_save_without_replacing_celsius_cache_and_hide_keeps_city() {
    let mut state = ready();
    state.open(crate::state::Page::Settings);
    state.focus = Target::Widget(1);
    state.activate();
    let place = crate::weather::Location {
        name: "Vienna".into(),
        region: "Vienna".into(),
        country: "Austria".into(),
        latitude: 48.2085,
        longitude: 16.3721,
    };
    state.preferences.weather_location = Some(place.clone());
    state.weather.cache = Some(crate::weather::Cache {
        key: place.key(),
        updated_at: 1000,
        reading: crate::weather::Reading {
            sun: None,
            temperature: 14.2,
            code: 2,
            is_day: true,
        },
    });
    state.focus = Target::WeatherTemperature;
    state.activate();
    assert_eq!(state.focus, Target::Choice(0));
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert_eq!(
        state.preferences.temperature_unit,
        crate::weather::Unit::Fahrenheit
    );
    assert_eq!(
        state.weather.cache.as_ref().unwrap().reading.temperature,
        14.2
    );
    assert_eq!(state.focus, Target::WeatherTemperature);
    state.focus = Target::WeatherToggle;
    state.activate();
    assert!(!state.preferences.show_weather);
    assert_eq!(state.preferences.weather_location, Some(place));
}

#[test]
fn home_weather_setup_returns_through_the_widgets_weather_parent() {
    use crate::state::{Subpage, WIDGETS_SECTION};
    let mut state = ready();
    state.focus = Target::WeatherConfigure;
    state.activate();
    assert_eq!(state.page, Page::Settings);
    assert_eq!(state.section, WIDGETS_SECTION);
    assert_eq!(state.subpage, Some(Subpage::WeatherLocation));
    assert!(state.text_input_active());
    state.back();
    state.back();
    assert_eq!(state.subpage, Some(Subpage::Weather));
    assert_eq!(state.focus, Target::WeatherLocation);
    state.back();
    assert!(state.subpage.is_none());
    assert_eq!(state.focus, Target::Widget(1));
}

#[test]
fn removing_weather_location_clears_cache_and_cancels_pending_results() {
    let mut state = ready();
    state.open(crate::state::Page::Settings);
    state.focus = Target::Widget(1);
    state.activate();
    state.focus = Target::WeatherLocation;
    state.activate();
    state.focus = Target::WeatherClear;
    let effects = state.activate();
    assert_eq!(
        effects,
        vec![
            Effect::Save,
            Effect::Weather(crate::weather::Command::ClearCache)
        ]
    );
    assert!(state.weather.cache.is_none());
    assert!(state.preferences.weather_location.is_none());
    assert_eq!(state.focus, Target::WeatherSearch);
}

#[test]
fn language_selection_restores_its_parent_and_is_saved_with_preferences() {
    let mut state = ready();
    state.open(crate::state::Page::Settings);
    state.focus = Target::Setting(4);
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.subpage, Some(crate::state::Subpage::Language));
    assert_eq!(
        state.focus,
        Target::Choice(crate::i18n::Language::En.index())
    );
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert_eq!(state.preferences.language, crate::i18n::Language::Nl);
    assert_eq!(state.focus, Target::Setting(4));
    state.activate();
    state.direction(Direction::Left, &controls(&state));
    assert_eq!(state.focus, Target::Setting(4));
}

#[test]
fn remote_can_select_and_restore_every_language() {
    for (index, language) in crate::i18n::Language::ALL.into_iter().enumerate() {
        let mut state = ready();
        state.open(crate::state::Page::Settings);
        state.focus = Target::Setting(4);
        state.activate();
        for _ in 0..crate::i18n::Language::En.index() {
            state.direction(Direction::Up, &controls(&state));
        }
        for _ in 0..index {
            state.direction(Direction::Down, &controls(&state));
        }
        assert_eq!(state.focus, Target::Choice(index));
        let focused = controls(&state)
            .into_iter()
            .find(|control| control.target == state.focus)
            .unwrap();
        let viewport = state.content_viewport();
        assert!(focused.rect.y >= viewport.y);
        assert!(focused.rect.y + focused.rect.h <= viewport.y + viewport.h);
        assert_eq!(state.activate(), vec![Effect::Save]);
        assert_eq!(state.preferences.language, language);
        assert_eq!(state.focus, Target::Setting(4));

        let saved = serde_json::to_value(&state.preferences).unwrap();
        let mut restored = State::new(serde_json::from_value(saved).unwrap());
        restored.open(crate::state::Page::Settings);
        restored.focus = Target::Setting(4);
        restored.activate();
        assert_eq!(restored.preferences.language, language);
        assert_eq!(restored.focus, Target::Choice(index));
        restored.direction(Direction::Down, &controls(&restored));
        assert_eq!(
            restored.focus,
            Target::Choice((index + 1) % crate::i18n::Language::ALL.len())
        );
    }
}

#[test]
fn settings_order_language_sort_and_shortcut_empty_card() {
    use crate::{i18n::Language, renderer::Draw, state::Subpage};
    assert!(
        Language::ALL
            .windows(2)
            .all(|pair| pair[0].name().to_lowercase() < pair[1].name().to_lowercase())
    );
    for (index, language) in Language::ALL.into_iter().enumerate() {
        assert_eq!(language.index(), index);
    }
    let mut state = ready();
    state.open(crate::state::Page::Settings);
    let general = controls(&state);
    let language = general
        .iter()
        .find(|control| control.target == Target::Setting(4))
        .unwrap();
    let hidden = general
        .iter()
        .find(|control| control.target == Target::Setting(11))
        .unwrap();
    assert!(language.rect.y < hidden.rect.y);
    state.subpage = Some(Subpage::Shortcuts);
    let empty = scene(&state);
    assert!(empty.draw.iter().any(|draw| matches!(draw,
        Draw::Backdrop(_, rect, ..) if rect.x == view::SETTINGS_X
            && rect.y == view::SETTINGS_TOP + view::ROW_H + view::ROW_GAP && rect.h == 240.
    )));
    assert!(empty.draw.iter().any(|draw| matches!(draw,
        Draw::Text(label, _, style, ..) if label == state.preferences.language.t("noTiles") && style.size == 31.
    )));
    assert!(empty.draw.iter().any(|draw| matches!(draw,
        Draw::Text(label, ..) if label == "Press Configure to add your first shortcut."
    )));
    assert!(
        empty
            .controls
            .iter()
            .any(|control| control.target == Target::TileEditor)
    );
}

#[test]
fn unsupported_language_preferences_recover_without_losing_other_choices() {
    let preferences: Preferences = serde_json::from_value(
        json!({"language":"unknown","favorites":["netflix"],"show_app_names":false}),
    )
    .unwrap();
    assert_eq!(preferences.language, crate::i18n::Language::En);
    assert!(!preferences.show_app_names);
    assert_eq!(preferences.favorites, Some(vec!["netflix".into()]));
}

#[test]
fn unrelated_app_ids_and_titles_keep_their_installed_artwork() {
    for (id, title) in [
        ("com.tvweb.dashboard", "Glasshouse"),
        ("com.example.ardent", "Notes"),
        ("com.example.3ard", "Notes"),
        ("com.example.ard3", "Notes"),
        ("com.example.plexus", "Notes"),
        ("com.example.media", "Standard dashboard"),
        ("com.example.media", "netflixer"),
        ("com.example.media", "preNetflix"),
        ("amazon.alexa.view", "Amazon Alexa"),
        ("com.example.spotify", "Spotify tools"),
        ("com.example.media", "Plex"),
        ("com.example.browser", "Web Browser"),
        ("com.example.livetv", "Live TV"),
    ] {
        let app = App::new(id, title);
        assert_eq!(
            (app.brand.as_str(), app.category.as_str()),
            ("generic", "Utilities"),
            "An unrelated application must not be rebranded: {id} / {title}"
        );
    }
}

#[test]
fn curated_branding_does_not_use_title_words_or_partial_id_matches() {
    for (id, title) in [
        ("com.example.ard_more", "Notes"),
        ("com.example.more-ard", "Notes"),
        ("com.example.ARD", "Notes"),
        ("com.example.media", "ARD"),
        ("com.example.media", "(ARD)"),
        ("com.example.media", "ARD미디어테크"),
        ("com.example.media", "아에르데ARD"),
        ("com.example.media", "아에르데ARD미디어테크"),
        ("com.example.dashboard", "ARD"),
        ("com.example.media", "standard ardent ARD"),
    ] {
        let app = App::new(id, title);
        assert_eq!(
            (app.brand.as_str(), app.category.as_str()),
            ("generic", "Utilities"),
            "Only an exact app ID may select curated artwork: {id} / {title}"
        );
    }
}

#[test]
fn known_app_ids_preserve_the_curated_brands_and_categories() {
    let apps = demo_inventory().apps;
    let expected = [
        ("com.webos.app.livetv", "live-tv", "Streaming"),
        ("netflix", "netflix", "Streaming"),
        ("youtube.leanback.v4", "youtube", "Streaming"),
        ("com.apple.appletv", "apple-tv", "Streaming"),
        ("com.disney.disneyplus-prod", "disney-plus", "Streaming"),
        ("amazon", "prime-video", "Streaming"),
        ("plex", "plex", "Streaming"),
        ("spotify-beehive", "spotify", "Music"),
        ("org.jellyfin.webos", "jellyfin", "Streaming"),
        ("org.xbmc.kodi", "kodi", "Streaming"),
        ("com.limelight.webos", "moonlight", "Gaming"),
        ("org.webosbrew.hbchannel", "homebrew", "Utilities"),
        ("com.webos.app.browser", "browser", "Utilities"),
        ("com.arte.tv", "arte", "Streaming"),
        ("de.zdf.app.zdfm3", "zdf", "Streaming"),
        ("ard.mediathek", "das-erste", "Streaming"),
    ];
    for (id, brand, category) in expected {
        let app = apps.iter().find(|app| app.id == id).unwrap();
        assert_eq!(
            (app.brand.as_str(), app.category.as_str()),
            (brand, category),
            "Known app {id} must retain its curated brand and category"
        );
    }
}

#[test]
fn hdmi_plug_in_reports_the_cable_state_when_connected_only_means_present() {
    let inputs = normalize_inputs(&json!([
        {"id":"HDMI_1","label":"HDMI 1","port":1,"connected":true,"appId":"com.webos.app.hdmi1","hdmiPlugIn":false},
        {"id":"HDMI_2","label":"AVR-X1800H","port":2,"connected":true,"appId":"com.webos.app.hdmi2","hdmiPlugIn":true},
        {"id":"HDMI_3","label":"HDMI 3","port":3,"connected":true,"appId":"com.webos.app.hdmi3","hdmiPlugIn":false},
        {"id":"HDMI_4","label":"HDMI 4","port":4,"connected":true,"appId":"com.webos.app.hdmi4","hdmiPlugIn":false}
    ]))
    .unwrap();
    let connected: Vec<_> = inputs
        .iter()
        .map(|input| (input.id.as_str(), input.connected))
        .collect();
    assert_eq!(
        connected,
        [
            ("com.webos.app.hdmi1", Some(false)),
            ("com.webos.app.hdmi2", Some(true)),
            ("com.webos.app.hdmi3", Some(false)),
            ("com.webos.app.hdmi4", Some(false)),
        ]
    );
    assert_eq!(inputs[1].title, "AVR-X1800H");
    let preferences = Preferences::default();
    let visible: Vec<_> = inputs
        .iter()
        .filter(|input| preferences.input_visible(input))
        .map(|input| input.id.as_str())
        .collect();
    assert_eq!(visible, ["com.webos.app.hdmi2"]);
}

#[test]
fn inputs_without_hdmi_plug_in_keep_the_connected_report() {
    let inputs = normalize_inputs(&json!([
        {"appId":"com.webos.app.hdmi1","id":"HDMI_1","port":1,"connected":true},
        {"appId":"com.webos.app.hdmi2","id":"HDMI_2","port":2,"connected":false},
        {"appId":"com.webos.app.hdmi3","connected":true,"hdmiPlugIn":"yes"},
        {"appId":"com.webos.app.hdmi4"}
    ]))
    .unwrap();
    let connected: Vec<_> = inputs.iter().map(|input| input.connected).collect();
    assert_eq!(connected, [Some(true), Some(false), Some(true), None]);
}

#[test]
fn input_names_are_sanitized_and_old_preferences_load_without_them() {
    let old: Preferences = serde_json::from_value(json!({"background": "forest"})).unwrap();
    assert!(old.input_names.is_empty());
    let mut preferences: Preferences = serde_json::from_value(json!({
        "input_names": {
            "com.webos.app.hdmi1": "  Play\nStation\t5  ",
            "com.webos.app.hdmi2": "   ",
            "bad;id": "Ignored",
            "com.webos.app.hdmi3": "A".repeat(60),
        }
    }))
    .unwrap();
    preferences.sanitize();
    assert_eq!(
        preferences.input_names,
        BTreeMap::from([
            (
                "com.webos.app.hdmi1".to_string(),
                "PlayStation5".to_string()
            ),
            (
                "com.webos.app.hdmi3".to_string(),
                "A".repeat(crate::model::INPUT_NAME_LIMIT)
            ),
        ])
    );
    assert!(preferences.set_input_name("com.webos.app.hdmi2", " DENON "));
    assert!(!preferences.set_input_name("com.webos.app.hdmi2", "DENON"));
    assert_eq!(preferences.input_names["com.webos.app.hdmi2"], "DENON");
    assert!(preferences.set_input_name("com.webos.app.hdmi2", ""));
    assert!(!preferences.input_names.contains_key("com.webos.app.hdmi2"));
    assert!(!preferences.set_input_name("bad;id", "Name"));
}

#[test]
fn custom_input_names_keep_the_label_port_and_status_as_detail() {
    let mut preferences = Preferences::default();
    let input = Input {
        id: "com.webos.app.hdmi2".into(),
        title: "AVR-X1800H".into(),
        port: "HDMI 2".into(),
        connected: Some(true),
        signal: None,
    };
    let language = preferences.language;
    let status = |preferences: &Preferences| {
        view::input_status(&input, preferences.input_name(&input).is_some(), language)
    };
    assert_eq!(preferences.input_title(&input), "HDMI 2");
    assert_eq!(status(&preferences), "AVR-X1800H · Connected");
    preferences.set_input_name(&input.id, "DENON");
    assert_eq!(preferences.input_title(&input), "DENON");
    assert_eq!(status(&preferences), "AVR-X1800H · HDMI 2 · Connected");
    let plain = Input {
        title: "HDMI 2".into(),
        ..input.clone()
    };
    assert_eq!(
        view::input_status(&plain, true, language),
        "HDMI 2 · Connected"
    );
    assert_eq!(view::input_status(&plain, false, language), "Connected");
}

#[test]
fn remote_renames_and_resets_an_input_from_its_settings_page() {
    let mut state = ready();
    state.open(crate::state::Page::Settings);
    state.focus = Target::Widget(3);
    state.activate();
    let id = "com.webos.app.hdmi2".to_string();
    state.focus = Target::InputMode(id.clone());
    state.activate();
    assert_eq!(state.focus, Target::Choice(2));
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::InputRename);
    assert!(
        !controls(&state)
            .iter()
            .any(|c| c.target == Target::InputNameReset)
    );
    state.activate();
    assert!(matches!(state.overlay, Some(Overlay::Keyboard)));
    state.text("DENONX");
    state.delete();
    assert_eq!(state.back(), vec![Effect::Save]);
    assert_eq!(state.focus, Target::InputRename);
    assert_eq!(state.preferences.input_names[&id], "DENON");
    assert_eq!(
        state.preferences.input_title(&state.inventory.inputs[1]),
        "DENON"
    );
    // Dismissing an empty keyboard keeps the saved name.
    state.activate();
    assert!(state.finish_text_input().is_empty());
    assert_eq!(state.preferences.input_names[&id], "DENON");
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::InputNameReset);
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert!(state.preferences.input_names.is_empty());
    assert_eq!(state.focus, Target::InputRename);
    state.direction(Direction::Up, &controls(&state));
    assert_eq!(state.focus, Target::Choice(2));
    state.focus = Target::InputRename;
    state.direction(Direction::Left, &controls(&state));
    assert_eq!(state.focus, Target::InputMode(id));
    assert_eq!(state.subpage, Some(crate::state::Subpage::Inputs));
}

#[test]
fn keyboard_done_saves_a_typed_input_name() {
    let mut state = ready();
    state.open(crate::state::Page::Settings);
    state.focus = Target::Widget(3);
    state.activate();
    state.focus = Target::InputMode("com.webos.app.hdmi1".into());
    state.activate();
    state.focus = Target::InputRename;
    state.activate();
    state.text("PS5");
    state.focus = Target::Key(
        crate::state::KEYS
            .iter()
            .position(|k| *k == "Done")
            .unwrap(),
    );
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert_eq!(state.preferences.input_names["com.webos.app.hdmi1"], "PS5");
    assert!(state.overlay.is_none());
}

#[test]
fn home_lifecycle_replies_map_for_both_registration_versions() {
    use crate::platform::Lifecycle;
    // Lifecycle v2 (registerApp, in place of LG's Home executable) names it in "event".
    assert_eq!(
        Lifecycle::from_reply(&json!({"event": "relaunch", "reason": "hotKey"})),
        Some(Lifecycle::Relaunch)
    );
    assert_eq!(
        Lifecycle::from_reply(&json!({"event": "close"})),
        Some(Lifecycle::Close)
    );
    // Lifecycle v1 (registerNativeApp for a staged Home manifest on webOS 10/11) in "message".
    assert_eq!(
        Lifecycle::from_reply(&json!({
            "parameters": {"launchType": "homeKey", "activateType": "home"},
            "returnValue": true,
            "message": "relaunch"
        })),
        Some(Lifecycle::Relaunch)
    );
    assert_eq!(
        Lifecycle::from_reply(&json!({"returnValue": true, "message": "close"})),
        Some(Lifecycle::Close)
    );
    // Acknowledgements and keep-alive need no action.
    for reply in [
        json!({"returnValue": true, "message": "registered"}),
        json!({"event": "registered"}),
        json!({"event": "pause", "reason": "keepAlive"}),
        json!({"returnValue": false, "errorText": "refused"}),
    ] {
        assert_eq!(Lifecycle::from_reply(&reply), None, "{reply}");
    }
}

#[test]
fn foreground_check_needs_focus_after_a_relaunch_from_the_background() {
    use crate::platform::ForegroundCheck;
    use std::time::{Duration, Instant};
    let start = Instant::now();
    let late = start + ForegroundCheck::TIMEOUT;

    // Relaunch while hidden, then only Shown (SDL emits it for SDL_ShowWindow itself): fails.
    let mut check = ForegroundCheck::default();
    check.relaunched(start, false);
    assert!(!check.failed(start + Duration::from_secs(1)));
    assert!(check.failed(late));
    // Reported once.
    assert!(!check.failed(late + Duration::from_secs(1)));

    // FocusGained or a remote key in time: no failure.
    let mut check = ForegroundCheck::default();
    check.relaunched(start, false);
    check.confirmed();
    assert!(!check.failed(late));

    // Already focused and visible (Home pressed on Home): nothing to check.
    let mut check = ForegroundCheck::default();
    check.relaunched(start, true);
    assert!(!check.failed(late));

    // A later relaunch from the background is checked again.
    check.relaunched(late, false);
    assert!(check.failed(late + ForegroundCheck::TIMEOUT));
}

#[test]
fn prewarm_waits_while_recents_replaces_the_page() {
    let mut state = ready();
    let mut jobs = vec![(Page::Settings, 0), (Page::Home, 0)];
    state.open_recents();
    state.recents.visibility = 1.;
    // As after the frame that showed it.
    state.dirty = false;
    // The Home scene would be Recents only, so preparing it now would be wasted.
    let apps_controls = |state: &mut State| {
        let current = std::mem::replace(&mut state.page, Page::Home);
        let found = controls(state)
            .iter()
            .any(|control| matches!(control.target, Target::App(_)));
        state.page = current;
        found
    };
    assert!(!apps_controls(&mut state));
    assert_eq!(crate::next_prewarm(&mut jobs, &state, true), None);
    assert_eq!(jobs, [(Page::Settings, 0), (Page::Home, 0)]);
    // Once Recents is gone the retained jobs run, in order.
    state.overlay = None;
    state.recents.visibility = 0.;
    state.dirty = false;
    assert!(apps_controls(&mut state));
    assert_eq!(crate::next_prewarm(&mut jobs, &state, false), None);
    assert_eq!(
        crate::next_prewarm(&mut jobs, &state, true),
        Some((Page::Home, 0))
    );
    assert_eq!(
        crate::next_prewarm(&mut jobs, &state, true),
        Some((Page::Settings, 0))
    );
    assert_eq!(crate::next_prewarm(&mut jobs, &state, true), None);
}

#[test]
fn failed_first_inventory_keeps_loading_until_retries_are_used_up() {
    let mut state = State::new(Preferences::default());
    state.loading = true;
    for attempt in 1..=crate::state::INVENTORY_RETRIES {
        assert_eq!(
            state.inventory_failed("The TV service timed out".into()),
            Some(crate::state::INVENTORY_RETRY_DELAY)
        );
        assert_eq!(state.inventory_failures, attempt);
        assert!(state.loading && state.error.is_none());
    }
    assert_eq!(
        state.inventory_failed("The TV service timed out".into()),
        None
    );
    assert!(!state.loading);
    assert_eq!(state.error.as_deref(), Some("The TV service timed out"));
    assert_eq!(state.first_content(), Target::Retry);
    // Retry by hand starts a fresh round of automatic retries.
    assert_eq!(state.inventory_failures, 0);
}

#[test]
fn inventory_success_resets_failures_and_later_failures_show_at_once() {
    let mut state = State::new(Preferences::default());
    state.loading = true;
    assert!(state.inventory_failed("timeout".into()).is_some());
    state.set_inventory(demo_inventory(), None);
    assert_eq!(state.inventory_failures, 0);
    state.loading = true;
    // Apps are on screen already: a failed refresh is reported, not retried behind them.
    assert_eq!(state.inventory_failed("timeout".into()), None);
    assert!(!state.loading && state.error.is_some());
    assert!(!state.inventory.apps.is_empty());
}

#[test]
fn tile_urls_need_a_host_the_request_client_accepts() {
    use crate::model::valid_tile_url;
    for url in [
        "http://homeassistant.local:8123/api/webhook/lamp",
        "http://ha/api/webhook/lamp",
        "http://my_nas.lan/hook",
        "https://example.com",
        "HTTPS://example.com/",
        "http://192.168.188.10:8123/api/webhook/lamp?on=1",
        "http://[::1]:8123/api/webhook/lamp",
        "http://[fd00::10]/hook",
        "http://user:secret@ha.local/hook",
        "http://ha.local:65535/",
    ] {
        assert!(valid_tile_url(url), "{url} should be valid");
    }
    for url in [
        "http://",
        "http:///",
        "http:////api",
        "http://:8123/foo",
        "http://user@/foo",
        "http://ha.local:/foo",
        "http://ha.local:0/foo",
        "http://ha.local:65536/foo",
        "http://ha.local:80a/foo",
        "http://ha.local:+80/foo",
        "http://ha.local:80:81/foo",
        "http://ha..local/foo",
        "http://-ha.local/foo",
        "http://ha%2elocal/foo",
        "http://999.1.1.1/foo",
        "http://192.168.1/foo",
        "http://[::1/foo",
        "http://[not-ip]/foo",
        "http://::1/foo",
        "http://ha.local/a b",
        "ftp://ha.local/foo",
    ] {
        assert!(!valid_tile_url(url), "{url} should be invalid");
    }
}

#[test]
fn custom_tiles_are_sanitized_and_get_stable_unique_ids() {
    use crate::model::{TileMethod, new_tile_id, valid_app_id};
    let mut preferences: Preferences = serde_json::from_value(json!({
        "custom_tiles": [
            {"id": "custom.lamp", "title": "  Lamp ", "url": " http://ha.local/api/webhook/a "},
            {"id": "custom.lamp", "title": "Duplicate", "url": "http://ha.local/b"},
            {"id": "custom.get", "title": "Get", "url": "HTTPS://ha.local/c", "method": "GET", "icon": "netflix"},
            {"id": "netflix", "title": "Not custom", "url": "http://ha.local/d"},
            {"id": "custom.ftp", "title": "FTP", "url": "ftp://ha.local/e"},
            {"id": "custom.space", "title": "Space", "url": "http://ha local/f"},
            {"id": "custom.bare", "title": "Bare", "url": "http://"},
            {"id": "custom.blank", "title": "  ", "url": "http://ha.local/g"}
        ]
    }))
    .unwrap();
    preferences.sanitize();
    let tiles: Vec<_> = preferences
        .custom_tiles
        .iter()
        .map(|tile| {
            (
                tile.id.as_str(),
                tile.title.as_str(),
                tile.url.as_str(),
                tile.method,
            )
        })
        .collect();
    assert_eq!(
        tiles,
        [
            (
                "custom.lamp",
                "Lamp",
                "http://ha.local/api/webhook/a",
                TileMethod::Post
            ),
            ("custom.get", "Get", "HTTPS://ha.local/c", TileMethod::Get),
        ]
    );
    // Only interface icons from the list are kept.
    assert!(preferences.custom_tiles[1].icon.is_empty());
    assert_eq!(preferences.custom_tiles[1].icon(), "weather-moon");
    assert_eq!(
        new_tile_id(&preferences.custom_tiles, "Lamp"),
        "custom.lamp-2"
    );
    assert_eq!(
        new_tile_id(&preferences.custom_tiles, "Licht Küche!"),
        "custom.licht-k-che"
    );
    assert_eq!(new_tile_id(&preferences.custom_tiles, "灯"), "custom.tile");
    assert!(valid_app_id(&new_tile_id(
        &preferences.custom_tiles,
        &"x".repeat(200)
    )));
}

#[test]
fn shortcut_settings_offer_qr_editing_and_a_test_remove_popup() {
    use crate::state::{Subpage, WIDGETS_SECTION};
    let mut state = ready();
    state.open(crate::state::Page::Settings);
    state.focus = Target::Widget(2);
    state.activate();
    assert_eq!(state.section, WIDGETS_SECTION);
    assert_eq!(state.subpage, Some(Subpage::Shortcuts));
    assert_eq!(
        controls(&state)
            .into_iter()
            .filter(|c| c.scrollable)
            .map(|c| c.target)
            .collect::<Vec<_>>(),
        vec![Target::TileEditor]
    );
    let tile = crate::model::CustomTile {
        id: "custom.lamp".into(),
        title: "Lamp".into(),
        url: "http://127.0.0.1:9/hook".into(),
        ..Default::default()
    };
    assert_eq!(state.replace_tiles(vec![tile.clone()]), vec![Effect::Save]);
    assert_eq!(state.subpage, Some(Subpage::Shortcuts));
    assert!(!state.text_input_active());
    assert_eq!(
        controls(&state)
            .into_iter()
            .filter(|c| c.scrollable)
            .map(|c| c.target)
            .collect::<Vec<_>>(),
        vec![Target::Tile(tile.id.clone()), Target::TileEditor]
    );
    assert_eq!(
        state.hit_test(&controls(&state), 800., view::SETTINGS_TOP + 140.),
        Some(Target::Tile(tile.id.clone()))
    );
    let page = scene(&state);
    // An unfocused shortcut is a glass description row with a primary (white) title.
    assert!(page.draw.iter().any(|draw| matches!(draw, crate::renderer::Draw::Text(label, _, _, color, _) if label == "Lamp" && *color == crate::renderer::WHITE)));
    assert!(page.draw.iter().any(|draw| matches!(draw,
        crate::renderer::Draw::Backdrop(_, rect, radius, _)
            if rect.x == view::SETTINGS_X && rect.h == view::DETAIL_ROW_H && *radius == 16.
    )));
    assert!(
        !page
            .controls
            .iter()
            .any(|control| matches!(control.target, Target::TileRemove(_)))
    );
    assert!(!page.draw.iter().any(|draw| matches!(draw, crate::renderer::Draw::TintedImage(path, rect, ..) if path.ends_with("right.lvec") && rect.x >= view::SETTINGS_X && rect.y >= view::SETTINGS_TOP)));
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Tile(tile.id.clone()));
    state.activate();
    assert_eq!(state.focus, Target::TileTest(tile.id.clone()));
    let popup = scene(&state);
    assert!(popup.modal.is_some());
    assert_eq!(
        popup
            .controls
            .iter()
            .map(|control| control.target.clone())
            .collect::<Vec<_>>(),
        vec![
            Target::TileTest(tile.id.clone()),
            Target::TileRemove(tile.id.clone())
        ]
    );
    assert!(
        matches!(state.activate().as_slice(), [Effect::FireWebhook(sent)] if sent.id == tile.id)
    );
    state.back();
    assert_eq!(state.focus, Target::Tile(tile.id.clone()));
    state.activate();
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::TileRemove(tile.id.clone()));
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert!(state.preferences.custom_tiles.is_empty());
    assert!(
        !state
            .preferences
            .favorites(&state.inventory.apps)
            .contains(&tile.id)
    );
    assert!(!state.inventory.apps.iter().any(|app| app.id == tile.id));
    assert_eq!(state.subpage, Some(Subpage::Shortcuts));
    assert_eq!(state.focus, Target::TileEditor);
    state.back();
    assert_eq!(state.subpage, None);
    assert_eq!(state.focus, Target::Widget(2));
}

#[test]
fn widgets_group_existing_settings_and_restore_each_parent_without_changing_preferences() {
    use crate::state::{SECTION_COUNT, Subpage, WIDGETS_SECTION};
    let mut state = ready();
    state.preferences.greeting_name = "Ana".into();
    state.preferences.show_weather = false;
    state.preferences.weather_location = Some(crate::weather::Location {
        name: "Vienna".into(),
        region: "Vienna".into(),
        country: "Austria".into(),
        latitude: 48.2085,
        longitude: 16.3721,
    });
    state.replace_tiles(vec![crate::model::CustomTile {
        id: "custom.lamp".into(),
        title: "Lamp".into(),
        url: "http://127.0.0.1:9/hook".into(),
        icon: "power".into(),
        ..Default::default()
    }]);
    let saved = serde_json::to_value(&state.preferences).unwrap();
    state.open(crate::state::Page::Settings);
    assert_eq!(SECTION_COUNT, 4);
    assert!(
        !controls(&state)
            .iter()
            .any(|control| control.target == Target::Setting(6))
    );
    // Settings opens on its first section.
    assert_eq!(state.focus, Target::Sidebar(0));
    state.direction(Direction::Down, &controls(&state));
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Sidebar(WIDGETS_SECTION));
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.focus, Target::Widget(4));
    let widget_order: Vec<_> = controls(&state)
        .into_iter()
        .filter_map(|control| match control.target {
            Target::Widget(index) => Some(index),
            _ => None,
        })
        .collect();
    // App artwork (5) is a toggle after the subpages.
    assert_eq!(widget_order, [4, 0, 1, 2, 3]);
    for (index, subpage) in [
        Subpage::Greeting,
        Subpage::Weather,
        Subpage::Shortcuts,
        Subpage::Inputs,
    ]
    .into_iter()
    .enumerate()
    {
        state.focus = Target::Widget(index);
        state.direction(Direction::Right, &controls(&state));
        assert_eq!(state.subpage, Some(subpage));
        state.back();
        assert!(state.subpage.is_none());
        assert_eq!(state.focus, Target::Widget(index));
    }
    state.direction(Direction::Left, &controls(&state));
    assert_eq!(state.focus, Target::Sidebar(WIDGETS_SECTION));
    assert_eq!(serde_json::to_value(&state.preferences).unwrap(), saved);
    state.inventory.inputs.clear();
    state.focus = Target::Widget(3);
    state.activate();
    assert_eq!(state.subpage, Some(Subpage::Inputs));
    // Inputs opens on its Show widget toggle; without inputs, Down reaches Refresh.
    assert_eq!(state.focus, Target::InputsToggle);
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Setting(3));
    assert_eq!(state.activate(), vec![Effect::Reload]);
    state.direction(Direction::Left, &controls(&state));
    assert_eq!(state.focus, Target::Widget(3));
}

#[test]
fn companion_removal_updates_the_shortcut_list_on_close() {
    use crate::state::Subpage;
    let mut state = ready();
    let tile = crate::model::CustomTile {
        id: "custom.lamp".into(),
        title: "Lamp".into(),
        url: "http://127.0.0.1:9/hook".into(),
        ..Default::default()
    };
    state.replace_tiles(vec![tile.clone()]);
    state.open(crate::state::Page::Settings);
    state.focus = Target::Widget(2);
    state.activate();
    state.activate();
    assert!(matches!(state.overlay, Some(Overlay::TileEditor)));
    state.replace_tiles(vec![]);
    assert_eq!(state.subpage, Some(Subpage::Shortcuts));
    assert_eq!(state.focus, Target::CloseDialog);
    state.back();
    assert_eq!(state.focus, Target::TileEditor);
    assert!(
        !state
            .preferences
            .favorites(&state.inventory.apps)
            .contains(&tile.id)
    );
    state.back();
    assert_eq!(state.focus, Target::Widget(2));
}

#[test]
fn widgets_are_localized_without_legacy_sidebar_entries() {
    use crate::state::WIDGETS_SECTION;
    for language in crate::i18n::Language::ALL {
        let mut state = ready();
        state.preferences.language = language;
        state.open(crate::state::Page::Settings);
        state.pointer_focus(Target::Sidebar(WIDGETS_SECTION));
        let page = scene(&state);
        assert_eq!(
            page.controls
                .iter()
                .filter(|c| matches!(c.target, Target::Sidebar(_)))
                .count(),
            4
        );
        assert_eq!(
            page.controls
                .iter()
                .filter(|c| matches!(c.target, Target::Widget(_)))
                .count(),
            5
        );
        for key in [
            "widgets",
            "greeting",
            "weather",
            "shortcuts",
            "inputs",
            "date",
        ] {
            assert!(page.draw.iter().any(|draw| matches!(draw, crate::renderer::Draw::Text(label, ..) if label == language.t(key))), "{key}/{language:?}");
        }
        state.pointer_focus(Target::Sidebar(0));
        assert!(!scene(&state).draw.iter().any(|draw| matches!(draw, crate::renderer::Draw::Text(label, ..) if label == language.t("greeting"))));
    }
}

#[test]
fn shortcut_list_scroll_and_pointer_focus_preserve_control_positions() {
    let mut state = ready();
    state.replace_tiles(
        (0..20)
            .map(|index| crate::model::CustomTile {
                id: format!("custom.tile-{index}"),
                title: format!("Tile {index}"),
                url: "http://127.0.0.1:9/hook".into(),
                ..Default::default()
            })
            .collect(),
    );
    state.open(crate::state::Page::Settings);
    state.focus = Target::Widget(2);
    state.activate();
    for _ in 0..20 {
        state.direction(Direction::Down, &controls(&state));
    }
    assert_eq!(state.focus, Target::Tile("custom.tile-19".into()));
    assert!(state.scroll > 0.);
    let before: Vec<_> = controls(&state)
        .iter()
        .map(|c| (c.target.clone(), c.rect.y))
        .collect();
    let scroll = state.scroll;
    let target = controls(&state)
        .into_iter()
        .find(|c| matches!(c.target, Target::Tile(_)) && c.rect.y > state.content_viewport().y)
        .unwrap();
    state.pointer_focus(target.target);
    assert_eq!(state.scroll, scroll);
    assert_eq!(
        before,
        controls(&state)
            .iter()
            .map(|c| (c.target.clone(), c.rect.y))
            .collect::<Vec<_>>()
    );
}

#[test]
fn settings_lists_keep_bottom_padding_for_remote_and_wheel_scrolling() {
    use crate::state::{SETTINGS_LIST_FADE, SETTINGS_SAFE_BOTTOM, Subpage};
    for backgrounds in [false, true] {
        let mut state = ready();
        state.open(crate::state::Page::Settings);
        if backgrounds {
            state.set_wallpapers(
                (1..=8)
                    .map(|index| crate::wallpapers::Wallpaper {
                        id: format!("custom-{index:032x}"),
                        directory: std::path::PathBuf::from(".build/padding-fixture"),
                        has_original: true,
                        blurred: [false; 2],
                    })
                    .collect(),
            );
        }
        state.section = if backgrounds { 1 } else { 0 };
        state.subpage = if backgrounds {
            None
        } else {
            Some(Subpage::Language)
        };
        let last = if backgrounds {
            Target::Background(state.background_count() - 1)
        } else {
            Target::Choice(crate::i18n::Language::ALL.len() - 1)
        };
        state.focus = last.clone();
        state.reveal_focus(&controls(&state));
        let bottom = SETTINGS_SAFE_BOTTOM - SETTINGS_LIST_FADE;
        let rect = controls(&state)
            .into_iter()
            .find(|c| c.target == last)
            .unwrap()
            .rect;
        assert!(rect.y + rect.h <= bottom + 0.01);
        state.wheel_scroll(&controls(&state), Some((1000., 600.)), 0, -1000);
        let rect = controls(&state)
            .into_iter()
            .find(|c| c.target == last)
            .unwrap()
            .rect;
        if backgrounds {
            assert!((rect.y + rect.h - bottom).abs() < 0.01);
        } else {
            // English and Dutch fit without scrolling.
            assert_eq!(state.scroll, 0.);
        }
    }
}

#[test]
fn language_list_scrolls_under_fixed_ui_and_fades_beneath_the_title_line() {
    use crate::{renderer::Draw, state::Subpage};
    let mut state = ready();
    state.open(crate::state::Page::Settings);
    state.subpage = Some(Subpage::Language);
    state.scroll = 180.;
    let rendered = scene(&state);
    // No clipping or side fades: rows fade beneath the title line and above the bottom safe line.
    assert!(
        !rendered
            .draw
            .iter()
            .any(|draw| matches!(draw, Draw::Clip(Some(_))))
    );
    assert!(rendered.draw.iter().any(|draw| matches!(
        draw,
        Draw::EdgeFade(Some(fade)) if fade[1] <= view::SETTINGS_TOP
            && fade[3] == crate::state::SETTINGS_SAFE_BOTTOM
    )));
    let first = rendered
        .controls
        .iter()
        .find(|control| control.target == Target::Choice(0))
        .unwrap();
    assert!(first.rect.y < state.content_viewport().y);
    let last_language = rendered
        .draw
        .iter()
        .rposition(|draw| {
            matches!(draw,
                Draw::Text(label, ..) if label == crate::i18n::Language::ALL.last().unwrap().name()
            )
        })
        .unwrap();
    for key in ["interface", "settings"] {
        let label = state.preferences.language.t(key);
        let fixed = rendered
            .draw
            .iter()
            .position(|draw| {
                matches!(draw,
                    Draw::Text(text, ..) if text == label
                )
            })
            .unwrap();
        assert!(fixed > last_language);
    }
    // The fixed sidebar keeps pointer priority over rows scrolled beneath it.
    let sidebar = rendered
        .controls
        .iter()
        .find(|control| control.target == Target::Sidebar(0))
        .unwrap();
    let (x, y) = sidebar.rect.center();
    assert_eq!(
        state.hit_test(&rendered.controls, x, y),
        Some(Target::Sidebar(0))
    );
}

#[test]
fn grid_rows_scroll_beneath_the_clock_and_fade_out_there() {
    use crate::renderer::Draw;
    let mut state = ready();
    state.open(Page::Home);
    state.scroll = 600.;
    let rendered = scene(&state);
    assert!(
        !rendered
            .draw
            .iter()
            .any(|draw| matches!(draw, Draw::Clip(Some(_))))
    );
    // One top fade, ending below the clock line.
    let fades: Vec<_> = rendered
        .draw
        .iter()
        .filter_map(|draw| match draw {
            Draw::EdgeFade(Some(fade)) => Some(*fade),
            _ => None,
        })
        .collect();
    assert_eq!(fades.len(), 1);
    assert!(
        fades[0][1] > view::HEADER.y + view::HEADER.h
            && fades[0][1] <= crate::state::GRID_TOP_LIMIT
    );
    let card = rendered
        .draw
        .iter()
        .position(|draw| {
            matches!(draw,
                Draw::Shadow(rect, ..) if rect.y < view::HEADER.y + view::HEADER.h && rect.y + rect.h > 0.
            )
        })
        .expect("cards remain drawn until they leave the screen");
    let clock = rendered
        .draw
        .iter()
        .rposition(|draw| matches!(draw, Draw::Text(_, rect, ..) if rect.y < view::HEADER.y + view::HEADER.h))
        .unwrap();
    assert!(card < clock, "The clock is drawn above scrolled cards");
}

#[test]
fn clock_date_and_home_input_descriptions_use_tvos_text_styles() {
    use crate::renderer::Draw;
    for language in crate::i18n::Language::ALL {
        let mut state = ready();
        state.preferences.language = language;
        let rendered = scene(&state);
        let now = time::OffsetDateTime::UNIX_EPOCH;
        // Clock and date share one line in Body (tvOS sizes; nothing below Caption 2).
        for (label, size) in [
            (
                language.clock(
                    now,
                    state.preferences.uses_24_hour(),
                    state.preferences.show_seconds,
                ),
                29.,
            ),
            (language.date_inline(now), 29.),
        ] {
            assert!(rendered.draw.iter().any(|draw| matches!(draw,
                Draw::Text(text, rect, style, ..) if text == &label && style.size == size && rect.h >= size
            )));
        }
        let input = state.inventory.inputs[0].clone();
        let description = view::input_status(
            &input,
            state.preferences.input_name(&input).is_some(),
            language,
        );
        // With inputs shown, a focused input glyph is captioned with its name and status
        // (Caption 2, nothing smaller).
        state.preferences.show_inputs = true;
        state.focus = Target::Input(input.id.clone());
        let focused = scene(&state);
        assert!(focused.draw.iter().any(|draw| matches!(draw,
            Draw::Text(text, _, style, ..) if text.contains(&description) && style.size >= 23.
        )));
    }
}

#[test]
fn one_screen_has_no_tab_bar_and_inputs_appear_only_when_enabled() {
    let mut state = ready();
    let rendered = scene(&state);
    // No Home/Apps/Settings tabs, TV-settings button, Search or Settings card: only apps.
    assert!(!rendered.draw.iter().any(|draw| matches!(draw,
        crate::renderer::Draw::Text(text, ..) if text == "Home")));
    let grid: Vec<_> = rendered
        .controls
        .iter()
        .filter(|control| matches!(control.target, Target::App(_)))
        .collect();
    assert_eq!(grid.first().unwrap().target, Target::App("netflix".into()));
    assert!(
        rendered
            .controls
            .iter()
            .any(|c| c.target == Target::SettingsIcon)
    );
    assert!(
        grid.iter().all(
            |control| control.rect.w == view::CARD_WIDTH && control.rect.h == view::CARD_HEIGHT
        )
    );
    assert!(
        !rendered
            .controls
            .iter()
            .any(|control| matches!(control.target, Target::Input(_)))
    );
    // Settings › Widgets › Inputs › Show widget adds small input glyphs to the top bar.
    state.open(Page::Settings);
    state.focus = Target::Widget(3);
    state.activate();
    assert_eq!(state.subpage, Some(crate::state::Subpage::Inputs));
    assert_eq!(state.focus, Target::InputsToggle);
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert!(state.preferences.show_inputs);
    state.open(Page::Home);
    let with_inputs = controls(&state);
    let first_input = with_inputs
        .iter()
        .find(|control| matches!(control.target, Target::Input(_)))
        .unwrap();
    let first_app = with_inputs
        .iter()
        .find(|control| control.target == Target::App("netflix".into()))
        .unwrap();
    // On the clock's line, left of the icons, small; the grid does not move.
    assert_eq!(first_input.rect.w, view::TOP_INPUT);
    assert!(first_input.rect.y < view::CLOCK_Y + 36.);
    let store = with_inputs
        .iter()
        .find(|control| control.target == Target::StoreIcon)
        .unwrap();
    assert!(first_input.rect.x + first_input.rect.w < store.rect.x);
    assert_eq!(first_app.rect.y, view::grid_y(&state));
    // Left from the first icon reaches the inputs; OK switches; long OK opens its settings.
    state.focus = Target::StoreIcon;
    state.direction(Direction::Left, &with_inputs);
    let Target::Input(id) = state.focus.clone() else {
        panic!("Left of the icons are the inputs: {:?}", state.focus);
    };
    assert_eq!(state.activate(), vec![Effect::Launch(id.clone())]);
    state.hold();
    assert_eq!(state.page, Page::Settings);
    assert_eq!(state.subpage, Some(crate::state::Subpage::Input(id)));
}

#[test]
fn greeting_visibility_toggle_persists_without_clearing_name_or_hiding_date() {
    use crate::{renderer::Draw, state::Subpage};
    let legacy: Preferences = serde_json::from_str(r#"{"greeting_name":"Ana"}"#).unwrap();
    assert!(legacy.show_greeting);
    let mut state = ready();
    state.preferences = legacy;
    state.open(crate::state::Page::Settings);
    state.focus = Target::Widget(0);
    state.activate();
    assert_eq!(state.subpage, Some(Subpage::Greeting));
    assert_eq!(state.focus, Target::GreetingToggle);
    let rendered = scene(&state);
    let toggle = rendered
        .controls
        .iter()
        .find(|control| control.target == Target::GreetingToggle)
        .unwrap();
    assert_eq!(toggle.rect.y, view::SETTINGS_TOP);
    assert!(rendered.draw.iter().any(|draw| matches!(draw,
        Draw::Text(label, ..) if label == state.preferences.language.t("showWidget")
    )));
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert!(!state.preferences.show_greeting);
    assert_eq!(state.preferences.greeting_name, "Ana");
    let saved = serde_json::to_string(&state.preferences).unwrap();
    let restored: Preferences = serde_json::from_str(&saved).unwrap();
    assert!(!restored.show_greeting);
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::GreetingName);
    state.focus = Target::GreetingToggle;
    state.direction(Direction::Left, &controls(&state));
    assert_eq!(state.focus, Target::Widget(0));
    state.open(crate::state::Page::Home);
    let hidden = scene(&state);
    let language = state.preferences.language;
    let now = time::OffsetDateTime::UNIX_EPOCH;
    let greeting = language.greeting(now.hour(), "Ana");
    assert!(
        !hidden
            .draw
            .iter()
            .any(|draw| matches!(draw, Draw::Text(label, ..) if label == &greeting))
    );
    assert!(
        hidden
            .draw
            .iter()
            .any(|draw| matches!(draw, Draw::Text(label, ..) if label == &language.date(now)))
    );
    state.open(crate::state::Page::Settings);
    state.focus = Target::Widget(0);
    state.activate();
    state.focus = Target::GreetingToggle;
    state.activate();
    assert!(state.preferences.show_greeting);
    state.open(crate::state::Page::Home);
    assert!(
        scene(&state)
            .draw
            .iter()
            .any(|draw| matches!(draw, Draw::Text(label, ..) if label == &greeting))
    );
}

#[test]
fn date_widget_settings_persist_and_restore_parent_focus() {
    use crate::{model::DateFormat, state::Subpage};
    let legacy: Preferences = serde_json::from_str("{}").unwrap();
    assert!(legacy.show_date);
    assert_eq!(legacy.date_format, DateFormat::Localized);
    let mut state = ready();
    state.open(crate::state::Page::Settings);
    state.focus = Target::Widget(4);
    state.activate();
    assert_eq!(state.subpage, Some(Subpage::Date));
    assert_eq!(state.focus, Target::DateToggle);
    let before = controls(&state);
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert!(!state.preferences.show_date);
    let after = controls(&state);
    for (a, b) in before.iter().zip(&after) {
        assert_eq!(a.target, b.target);
        assert_eq!((a.rect.x, a.rect.y), (b.rect.x, b.rect.y));
    }
    state.direction(Direction::Down, &after);
    assert_eq!(state.focus, Target::DateFormat);
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.subpage, Some(Subpage::DateFormat));
    assert_eq!(state.focus, Target::Choice(0));
    for index in 1..4 {
        state.direction(Direction::Down, &controls(&state));
        assert_eq!(state.focus, Target::Choice(index));
    }
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Choice(0));
    for _ in 0..3 {
        state.direction(Direction::Down, &controls(&state));
    }
    assert_eq!(state.focus, Target::Choice(3));
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert_eq!(state.preferences.date_format, DateFormat::YearMonthDay);
    assert_eq!(state.subpage, Some(Subpage::Date));
    assert_eq!(state.focus, Target::DateFormat);
    state.activate();
    assert_eq!(state.focus, Target::Choice(3));
    state.back();
    assert_eq!(state.focus, Target::DateFormat);
    state.back();
    assert_eq!(state.focus, Target::Widget(4));
    let saved: Preferences =
        serde_json::from_str(&serde_json::to_string(&state.preferences).unwrap()).unwrap();
    assert!(!saved.show_date);
    assert_eq!(saved.date_format, DateFormat::YearMonthDay);
    state.open(crate::state::Page::Home);
    let date = saved
        .date_format
        .format(saved.language, time::OffsetDateTime::UNIX_EPOCH);
    assert!(
        !scene(&state)
            .draw
            .iter()
            .any(|draw| matches!(draw, crate::renderer::Draw::Text(label, ..) if label == &date))
    );
}

#[test]
fn date_formats_preserve_localized_default_and_pad_numeric_dates() {
    use crate::{i18n::Language, model::DateFormat};
    let now = time::Date::from_calendar_date(2026, time::Month::February, 3)
        .unwrap()
        .midnight()
        .assume_utc();
    for language in Language::ALL {
        assert_eq!(
            DateFormat::Localized.format(language, now),
            language.date(now)
        );
        assert_eq!(DateFormat::DayMonthYear.format(language, now), "03.02.2026");
        assert_eq!(DateFormat::MonthDayYear.format(language, now), "02/03/2026");
        assert_eq!(DateFormat::YearMonthDay.format(language, now), "2026-02-03");
        for key in ["date", "dateFormat", "dateLocalized"] {
            assert_ne!(language.t(key), key);
        }
    }
}

#[test]
fn update_card_shows_both_versions_and_explicitly_starts_installation() {
    use crate::{i18n::Language, renderer::Draw};
    for language in Language::ALL {
        let mut state = ready();
        state.preferences.language = language;
        state.open(crate::state::Page::Settings);
        state.section = crate::state::SYSTEM_SECTION;
        assert!(
            !controls(&state)
                .iter()
                .any(|control| control.target == Target::InstallUpdate)
        );
        state.available_update = Some("9.0.0".into());
        let rendered = scene(&state);
        for label in [
            language.t("updateAvailable").to_owned(),
            language.format("installUpdate", &[("version", "9.0.0")]),
            language.format(
                "updateVersions",
                &[("current", "test"), ("latest", "9.0.0")],
            ),
        ] {
            assert!(
                rendered
                    .draw
                    .iter()
                    .any(|draw| matches!(draw, Draw::Text(text, ..) if text == &label))
            );
        }
        state.focus = Target::Sidebar(crate::state::SYSTEM_SECTION);
        state.direction(Direction::Right, &rendered.controls);
        assert_eq!(state.focus, Target::InstallUpdate);
        assert_eq!(
            state.activate(),
            vec![Effect::UpdateDownload("9.0.0".into())]
        );
        let button = rendered
            .controls
            .iter()
            .find(|control| control.target == Target::InstallUpdate)
            .unwrap();
        let (x, y) = button.rect.center();
        assert_eq!(
            state.hit_test(&rendered.controls, x, y),
            Some(Target::InstallUpdate)
        );
        state.direction(Direction::Down, &rendered.controls);
        assert_eq!(state.focus, Target::Setting(3));
        state.focus = Target::InstallUpdate;
        state.direction(Direction::Left, &rendered.controls);
        assert_eq!(state.focus, Target::Sidebar(crate::state::SYSTEM_SECTION));
    }
}

#[test]
fn update_status_card_only_claims_latest_after_success_and_allows_single_flight_retry() {
    use crate::{renderer::Draw, update::Status};
    let mut state = ready();
    state.open(crate::state::Page::Settings);
    state.section = crate::state::SYSTEM_SECTION;
    state.focus = Target::CheckUpdates;
    assert_eq!(state.update_status, Status::Unchecked);
    assert!(!scene(&state).draw.iter().any(|draw| matches!(draw,
        Draw::Text(label, ..) if label == state.preferences.language.t("latestVersionInstalled")
    )));
    assert!(state.activate().is_empty());
    assert_eq!(state.update_status, Status::Checking);
    assert!(std::mem::take(&mut state.update_check_requested));
    state.activate();
    assert!(!state.update_check_requested);
    state.update_response(Err("offline".into()));
    assert_eq!(state.update_status, Status::Failed);
    assert!(!scene(&state).draw.iter().any(|draw| matches!(draw,
        Draw::Text(label, ..) if label == state.preferences.language.t("latestVersionInstalled")
    )));
    state.activate();
    assert!(std::mem::take(&mut state.update_check_requested));
    state.update_response(Ok(None));
    assert_eq!(state.update_status, Status::Checked);
    for key in ["latestVersionInstalled", "checkAgain"] {
        assert!(scene(&state).draw.iter().any(|draw| matches!(draw,
            Draw::Text(label, ..) if label == state.preferences.language.t(key)
        )));
    }
    state.activate();
    state.update_response(Ok(Some("9.0.0".into())));
    assert_eq!(state.focus, Target::InstallUpdate);
    state.update_response(Err("offline".into()));
    assert_eq!(state.available_update.as_deref(), Some("9.0.0"));
}

#[test]
fn shortcut_tiles_keep_apps_options_and_never_launch_or_uninstall_a_platform_app() {
    let tile = crate::model::CustomTile {
        id: "custom.lamp".into(),
        title: "Lamp".into(),
        url: "http://127.0.0.1:9/hook".into(),
        ..Default::default()
    };
    let mut state = ready();
    state.replace_tiles(vec![tile.clone()]);
    state.open(crate::state::Page::Home);
    state.focus = Target::App(tile.id.clone());
    assert_eq!(state.activate(), vec![Effect::FireWebhook(tile.clone())]);
    state.options();
    state.focus = Target::OpenApp;
    assert_eq!(state.activate(), vec![Effect::FireWebhook(tile.clone())]);

    state.open(crate::state::Page::Home);
    state.focus = Target::App(tile.id.clone());
    state.options();
    assert_eq!(state.focus, Target::Option(0));
    assert!(matches!(state.overlay, Some(Overlay::Options { .. })));
    assert!(
        !controls(&state)
            .iter()
            .any(|c| c.target == Target::Option(2))
    );
    state.focus = Target::OpenApp;
    assert_eq!(state.activate(), vec![Effect::FireWebhook(tile.clone())]);
    assert!(state.overlay.is_none());

    // A shortcut moves like any app.
    state.focus = Target::App(tile.id.clone());
    state.options();
    assert!(state.activate().is_empty());
    assert_eq!(state.moving.as_deref(), Some(tile.id.as_str()));
    state.back();
    state.focus = Target::App(tile.id.clone());
    state.options();
    state.focus = Target::Option(1);
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert!(state.preferences.hidden.contains(&tile.id));
    assert_eq!(state.preferences.custom_tile(&tile.id), Some(&tile));
}

#[test]
fn shortcut_tiles_alone_do_not_stop_inventory_retries() {
    let mut preferences: Preferences = serde_json::from_value(json!({
        "custom_tiles": [{"id": "custom.lamp", "title": "Lamp", "url": "http://ha.local/hook"}]
    }))
    .unwrap();
    preferences.sanitize();
    let mut state = State::new(preferences);
    state.loading = true;
    state.sync_custom_tiles();
    assert!(!state.inventory.apps.is_empty());
    assert_eq!(
        state.inventory_failed("timeout".into()),
        Some(crate::state::INVENTORY_RETRY_DELAY)
    );
    assert!(state.loading && state.error.is_none());
}

#[test]
fn shortcut_tiles_open_a_modal_editor_for_another_device() {
    use crate::state::WIDGETS_SECTION;
    let mut state = ready();
    state.open(crate::state::Page::Settings);
    state.focus = Target::Widget(2);
    state.activate();
    assert_eq!(state.focus, Target::TileEditor);
    assert!(state.activate().is_empty());
    assert!(matches!(state.overlay, Some(Overlay::TileEditor)));
    assert_eq!(state.focus, Target::CloseDialog);
    let modal = scene(&state);
    assert!(modal.modal.is_some());
    assert_eq!(modal.controls.len(), 1);
    assert_eq!(modal.controls[0].target, Target::CloseDialog);
    assert!(state.direction(Direction::Up, &modal.controls).is_empty());
    assert_eq!(state.focus, Target::CloseDialog);
    state.tile_editor = Some(Err("offline".into()));
    state.activate();
    assert!(state.overlay.is_none());
    assert!(state.tile_editor.is_none());
    assert_eq!(state.focus, Target::TileEditor);
    assert_eq!(state.section, WIDGETS_SECTION);
}

#[test]
fn tiles_edited_elsewhere_join_and_leave_home_like_tv_edits() {
    use crate::model::CustomTile;
    let tile = |id: &str, title: &str| CustomTile {
        id: id.into(),
        title: title.into(),
        url: "http://10.0.0.2/hook".into(),
        ..Default::default()
    };
    let mut state = ready();
    let before = state.preferences.app_order.clone().unwrap();
    let saved = state.replace_tiles(vec![
        tile("custom.gate", "Gate"),
        tile("custom.lights", "Lights"),
    ]);
    assert_eq!(saved, vec![Effect::Save]);
    // New shortcuts join the end of the user's order.
    let order = state.preferences.app_order.clone().unwrap();
    assert_eq!(&order[..before.len()], &before[..]);
    assert_eq!(&order[before.len()..], ["custom.gate", "custom.lights"]);
    assert!(
        state
            .inventory
            .apps
            .iter()
            .any(|app| app.id == "custom.gate")
    );
    state.preferences.hidden.push("custom.lights".into());
    // A rename keeps its place; a removal clears it from the order and hidden apps.
    state.replace_tiles(vec![tile("custom.gate", "Garage")]);
    assert_eq!(state.preferences.custom_tiles[0].title, "Garage");
    let order = state.preferences.app_order.clone().unwrap();
    assert_eq!(&order[before.len()..], ["custom.gate"]);
    assert!(state.preferences.hidden.is_empty());
    assert!(
        !state
            .inventory
            .apps
            .iter()
            .any(|app| app.id == "custom.lights")
    );
    let app = state
        .inventory
        .apps
        .iter()
        .find(|app| app.id == "custom.gate")
        .unwrap();
    assert_eq!(app.title, "Garage");
}

#[test]
fn update_card_shows_progress_restores_retry_and_ignores_presses_while_busy() {
    use crate::update::{InstallStatus, Response, Status};
    let mut state = ready();
    state.open(crate::state::Page::Settings);
    state.section = crate::state::SYSTEM_SECTION;
    state.focus = Target::CheckUpdates;
    assert!(state.activate().is_empty());
    assert_eq!(state.update_status, Status::Checking);
    assert!(
        state.activate().is_empty(),
        "no second check while one runs"
    );
    state.update_response(Ok(Some("9.9.9".into())));
    assert!(scene(&state).draw.iter().any(|draw| {
        matches!(draw, crate::renderer::Draw::Text(text, ..) if text == "Install 9.9.9")
    }));
    let initial = scene(&state);
    assert!(
        !initial
            .controls
            .iter()
            .any(|control| control.target == Target::Setting(8))
    );
    assert_eq!(
        state.activate(),
        vec![Effect::UpdateDownload("9.9.9".into())]
    );
    assert_eq!(state.update, InstallStatus::Downloading(0));
    assert!(state.activate().is_empty());
    state.install_response(Response::Progress(45));
    assert!(scene(&state).draw.iter().any(|draw| {
        matches!(draw, crate::renderer::Draw::Text(text, ..) if text == "Downloading 45 %")
    }));
    let geometry = |items: Vec<crate::state::Control>| {
        items
            .into_iter()
            .map(|item| {
                (
                    item.target,
                    item.rect.x,
                    item.rect.y,
                    item.rect.w,
                    item.rect.h,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        geometry(controls(&state)),
        geometry(initial.controls),
        "progress must not move hit targets"
    );
    state.install_response(Response::Installing("9.9.9".into()));
    assert!(state.activate().is_empty());
    state.pressed = Some(Target::InstallUpdate);
    state.install_response(Response::Finished(Err("disk full".into())));
    assert_eq!(state.focus, Target::CheckUpdates);
    assert!(state.pressed.is_none());
    assert!(state.available_update.is_none());
    assert!(!state.update_busy());
    state.activate();
    assert_eq!(state.update_status, Status::Checking);
    assert_eq!(state.update, InstallStatus::Idle);
    state.install_response(Response::Finished(Ok(())));
    assert_eq!(state.update_status, Status::Unchecked);
}

#[test]
fn removed_general_preferences_load_and_their_rows_stay_gone() {
    // Files still carrying the removed Sounds, Focus tilt or Favorite badges preferences load
    // and drop them; there are no heart badges anywhere.
    let old: Preferences = serde_json::from_value(json!({
        "favorites": ["netflix"], "sounds": true, "focus_tilt": true, "favorite_badges": true
    }))
    .unwrap();
    let saved = serde_json::to_value(&old).unwrap();
    let saved = saved.as_object().unwrap();
    for key in ["sounds", "focus_tilt", "favorite_badges"] {
        assert!(!saved.contains_key(key), "{key}");
    }
    let mut state = ready();
    let hearts = |state: &State| {
        scene(state)
            .draw
            .iter()
            .filter(|draw| matches!(draw, crate::renderer::Draw::TintedImage(path, ..) if path.ends_with("heart-filled.lvec")))
            .count()
    };
    assert_eq!(hearts(&state), 0);
    state.open(Page::Settings);
    // Focus tilt (8), Sounds (9) and Favorite badges (10) never come back.
    for removed in [8, 9, 10] {
        assert!(
            !controls(&state)
                .iter()
                .any(|c| c.target == Target::Setting(removed))
        );
    }
}

#[test]
fn home_dims_its_wallpaper_constantly_and_never_per_app() {
    use crate::renderer::Draw;
    // Files saved with the removed App artwork preference still load and drop it.
    let old: Preferences =
        serde_json::from_value(json!({"favorites": ["netflix"], "app_artwork": true})).unwrap();
    assert!(
        !serde_json::to_value(&old)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("app_artwork")
    );
    let mut state = ready();
    state.open(Page::Home);
    let dims = |state: &State| {
        scene(state)
            .draw
            .iter()
            .filter_map(|draw| match draw {
                Draw::Shape(rect, 0., 0., color)
                    if rect.x <= 0. && rect.y <= 0. && rect.w >= 1920. && rect.h >= 1080. =>
                {
                    Some(*color)
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let first = dims(&state);
    assert_eq!(
        first.len(),
        1,
        "one full-screen dim behind the shelf and grid"
    );
    assert_eq!(first[0][3], view::HOME_DIM);
    // It lies directly on the wallpaper, so the renderer folds it into the wallpaper's pass.
    let draws = scene(&state).draw;
    let at = draws
        .iter()
        .position(|draw| matches!(draw, Draw::Shape(rect, 0., 0., _) if rect.w >= 1920.))
        .unwrap();
    assert!(matches!(&draws[at - 1], Draw::Image(_, rect, ..) if rect.w == 1920.));
    // The same for every focused app, in any row: nothing reacts to the app.
    for target in [state.grid()[1].clone(), state.grid()[13].clone()] {
        state.focus = target;
        state.sync_motion();
        assert_eq!(dims(&state), first);
        assert!(!scene(&state).draw.iter().any(|draw| matches!(draw,
            Draw::AppImage(_, rect, ..) | Draw::TintedImage(_, rect, ..) if rect.w == 1920. && rect.y == 0. && rect.h < 1080.)));
    }
    // Widgets lists only its five subpages; the App artwork row is gone.
    state.open(Page::Settings);
    state.section = crate::state::WIDGETS_SECTION;
    assert!(
        !controls(&state)
            .iter()
            .any(|c| c.target == Target::Widget(5))
    );
}

#[test]
fn the_top_bar_has_no_control_center_and_inputs_still_switch_from_it() {
    use crate::model::InputMode;
    let mut state = ready();
    state.open(Page::Home);
    assert_eq!(
        state.top_bar(),
        [Target::StoreIcon, Target::SettingsIcon],
        "Apps and Settings only"
    );
    // The inputs widget keeps its glyphs in the bar, and OK still switches to an input.
    state.preferences.show_inputs = true;
    let input = state.inventory.inputs[0].id.clone();
    state
        .preferences
        .input_modes
        .insert(input.clone(), InputMode::On);
    assert_eq!(state.top_bar()[0], Target::Input(input.clone()));
    state.focus = Target::Input(input.clone());
    assert_eq!(state.activate(), vec![Effect::Launch(input)]);
    assert!(state.overlay.is_none());
}

#[test]
fn screensaver_is_a_jumping_clock_on_black_and_any_input_ends_it() {
    use crate::renderer::Draw;
    use crate::state::Subpage;
    use std::time::{Duration, Instant};
    // Files saved with the removed Screensaver clock toggle still load and drop it.
    let old: Preferences =
        serde_json::from_value(json!({"favorites": ["netflix"], "screensaver_clock": false}))
            .unwrap();
    assert_eq!(old.screensaver_minutes, 10);
    assert!(
        !serde_json::to_value(&old)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("screensaver_clock")
    );
    let mut state = ready();
    state.preferences.language = crate::i18n::Language::Nl;
    assert_eq!(state.screensaver_after(), Some(Duration::from_secs(600)));
    state.start_screensaver();
    let rendered = scene(&state);
    assert!(
        rendered.controls.is_empty(),
        "Nothing behind it is reachable"
    );
    // Pure black and the two lines of text, nothing else: no wallpaper, no glass, no icons.
    assert!(matches!(&rendered.draw[0],
        Draw::Shape(rect, 0., 0., color) if rect.w >= 1920. && rect.h >= 1080. && *color == [0., 0., 0., 1.]));
    let texts: Vec<_> = rendered
        .draw
        .iter()
        .filter_map(|draw| match draw {
            Draw::Text(text, rect, style, color, _) => Some((text.clone(), *rect, *style, *color)),
            _ => None,
        })
        .collect();
    assert_eq!(rendered.draw.len(), 1 + texts.len());
    assert_eq!(texts.len(), 2);
    // The clock and date formats from Settings: Dutch has a 24-hour clock and a lowercase date.
    let now = time::OffsetDateTime::UNIX_EPOCH;
    let language = state.preferences.language;
    assert_eq!(texts[0].0, language.clock(now, true, false));
    assert_eq!(
        texts[1].0,
        state.preferences.date_format.format_inline(language, now)
    );
    assert_eq!(texts[1].0, texts[1].0.to_lowercase());
    // Light grey time at Title 1 scale; the date smaller and dimmer, beneath it.
    let (time_style, time_color) = (texts[0].2, texts[0].3);
    let (date_style, date_color) = (texts[1].2, texts[1].3);
    assert_eq!(time_style.size, 76.);
    assert!(date_style.size < time_style.size);
    assert!(time_color[0] > 0.6 && time_color[0] < 0.8 && time_color[0] == time_color[2]);
    assert!(date_color[0] < time_color[0]);
    assert!(texts[1].1.y > texts[0].1.y);
    // It fades in at first; between fades nothing animates (no continuous redraw).
    assert!(state.animating());
    let (_, seed) = state.screensaver.unwrap();
    state.screensaver = Some((Instant::now() - Duration::from_millis(2000), seed));
    assert!(!state.animating());
    // Every position keeps the whole block inside the safe area, and it moves over time.
    let assets = view::Assets {
        root: PathBuf::new(),
        version: "test".into(),
        metrics: BTreeMap::new(),
        artwork: crate::app_artwork::Service::new(false),
    };
    let time = language.clock(now, true, false);
    let date = state.preferences.date_format.format_inline(language, now);
    let mut places = std::collections::HashSet::new();
    for seed in [1u64, 42, u64::MAX] {
        for step in 0..200 {
            let block = view::screensaver_block(&assets, &time, &date, seed, step);
            assert!(block.x >= view::SAFE_X && block.x + block.w <= 1920. - view::SAFE_X);
            assert!(block.y >= 60. && block.y + block.h <= 1020.);
            places.insert((block.x as i32, block.y as i32));
        }
    }
    assert!(places.len() > 500, "{} distinct places", places.len());
    // A jump every five seconds: the loop's tick marks the frame dirty only then.
    assert_eq!(state.screensaver_position_step().unwrap().1, 0);
    state.tick();
    assert_eq!(state.screensaver_step, 0);
    let (_, seed) = state.screensaver.unwrap();
    state.screensaver = Some((Instant::now() - Duration::from_millis(5100), seed));
    state.dirty = false;
    state.tick();
    assert!(state.dirty, "jumped");
    assert_eq!(state.screensaver_step, 1);
    let moved = scene(&state);
    assert!(!matches!((&moved.draw[1], &rendered.draw[1]),
        (Draw::Text(_, a, ..), Draw::Text(_, b, ..)) if a.x == b.x && a.y == b.y));
    // Any input only ends it.
    assert!(state.stop_screensaver());
    assert!(!state.stop_screensaver());
    assert!(!scene(&state).controls.is_empty());
    state.preferences.language = crate::i18n::Language::En;
    // Settings › General › Screensaver: Off disables the timer.
    state.open(Page::Settings);
    state.focus = Target::Setting(12);
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.subpage, Some(Subpage::Screensaver));
    // Off, 1, 5, 10 (selected), 15 and 30 minutes.
    assert_eq!(state.focus, Target::Choice(3));
    state.direction(Direction::Up, &controls(&state));
    state.direction(Direction::Up, &controls(&state));
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert_eq!(state.preferences.screensaver_minutes, 1);
    assert_eq!(
        state.screensaver_after(),
        Some(std::time::Duration::from_secs(60))
    );
    state.focus = Target::Setting(12);
    state.activate();
    assert_eq!(
        state.focus,
        Target::Choice(1),
        "The saved minute is selected"
    );
    state.direction(Direction::Up, &controls(&state));
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert_eq!(state.preferences.screensaver_minutes, 0);
    assert_eq!(state.screensaver_after(), None);
    assert_eq!(state.focus, Target::Setting(12));
    // The Screensaver clock row is gone: the clock is the screensaver.
    assert!(
        !controls(&state)
            .iter()
            .any(|c| c.target == Target::Setting(13))
    );
}

#[test]
fn appearance_defaults_to_dark_and_automatic_follows_the_sun_at_the_weather_location() {
    use crate::model::Appearance;
    use crate::renderer::Draw;
    let old: Preferences =
        serde_json::from_value(json!({"appearance": "sepia", "favorites": []})).unwrap();
    assert_eq!(old.appearance, Appearance::Dark);
    let mut state = ready();
    let at = |hour: u8, minute: u8| {
        time::OffsetDateTime::UNIX_EPOCH
            .replace_hour(hour)
            .unwrap()
            .replace_minute(minute)
            .unwrap()
    };
    let scrim = |scene: &view::Scene| {
        scene.draw.iter().any(|draw| {
            matches!(draw, Draw::Shape(rect, _, _, color) if rect.w == 1920. && color[0] > 0.9 && color[3] == 0.5)
        })
    };
    // Dark (the default) leaves the scene untouched.
    assert!(!state.light(at(12, 0)));
    assert!(!scrim(&scene(&state)));
    // Light brightens it: a scrim, and white text turns dark.
    state.preferences.appearance = Appearance::Light;
    let light = scene(&state);
    assert!(scrim(&light));
    assert!(light.draw.iter().any(|draw| matches!(draw,
        Draw::Text(_, _, _, color, _) if color[0] < 0.2)));
    // Automatic without a forecast: light from 07:00 to 21:00.
    state.preferences.appearance = Appearance::Automatic;
    assert!(!state.light(at(6, 59)));
    assert!(state.light(at(7, 0)));
    assert!(state.light(at(20, 59)));
    assert!(!state.light(at(21, 0)));
    // With a forecast: today's sunrise and sunset at the location.
    let place = crate::weather::Location {
        name: "Wenen".into(),
        region: "Wenen".into(),
        country: "Oostenrijk".into(),
        latitude: 48.2085,
        longitude: 16.3721,
    };
    state.preferences.weather_location = Some(place.clone());
    state.weather.cache = Some(crate::weather::Cache {
        key: place.key(),
        updated_at: 1000,
        reading: crate::weather::Reading {
            sun: Some([7 * 60 + 30, 17 * 60 + 5]),
            temperature: 9.,
            code: 0,
            is_day: false,
        },
    });
    assert!(!state.light(at(7, 29)));
    assert!(state.light(at(7, 30)));
    assert!(!state.light(at(17, 5)));
    // Settings › General › Appearance.
    state.open(Page::Settings);
    state.focus = Target::Setting(14);
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.subpage, Some(crate::state::Subpage::Appearance));
    assert_eq!(state.focus, Target::Choice(2));
    state.direction(Direction::Up, &controls(&state));
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert_eq!(state.preferences.appearance, Appearance::Light);
}

#[test]
fn forecasts_carry_todays_sunrise_and_sunset() {
    let reading = crate::weather::parse_reading(&json!({
        "current": {"temperature_2m": 9.0, "weather_code": 0, "is_day": 1},
        "daily": {"sunrise": ["2026-11-10T07:21"], "sunset": ["2026-11-10T16:44"]}
    }))
    .unwrap();
    assert_eq!(reading.sun, Some([7 * 60 + 21, 16 * 60 + 44]));
    // Missing or polar days have no times; the reading itself stays valid.
    let polar = crate::weather::parse_reading(&json!({
        "current": {"temperature_2m": -9.0, "weather_code": 0, "is_day": 1},
        "daily": {"sunrise": [null], "sunset": [null]}
    }))
    .unwrap();
    assert_eq!(polar.sun, None);
}

/// Where every card face is drawn: its rects, in drawing order.
fn card_faces(state: &State) -> Vec<[f32; 4]> {
    use crate::renderer::Draw;
    scene(state)
        .draw
        .iter()
        .filter_map(|draw| match draw {
            Draw::Image(path, rect, ..) | Draw::TintedImage(path, rect, ..)
                if path.contains("surfaces/") || path.contains("logos/") =>
            {
                Some([rect.x, rect.y, rect.w, rect.h])
            }
            Draw::AppImage(_, rect, ..) => Some([rect.x, rect.y, rect.w, rect.h]),
            _ => None,
        })
        .collect()
}

#[test]
fn move_mode_draws_no_jiggle_or_arrows_and_never_animates() {
    use crate::renderer::Draw;
    use std::time::Duration;
    let mut state = ready();
    state.open(Page::Home);
    let before = controls(&state);
    let faces = card_faces(&state);
    start_moving(&mut state, "netflix");
    // Every card is drawn exactly where it was: no jiggle offsets, now or later.
    assert_eq!(card_faces(&state), faces);
    assert!(!state.animating(), "move mode redraws nothing by itself");
    state.advance_motion(Duration::from_millis(700));
    assert_eq!(card_faces(&state), faces);
    // No arrow indicator; the moving card has its focus highlight and a stronger rim.
    let frame = scene(&state);
    assert!(!frame.draw.iter().any(|draw| matches!(draw,
        Draw::TintedImage(path, ..) if path.ends_with("left.lvec") || path.ends_with("arrow.lvec"))));
    let netflix = before
        .iter()
        .find(|c| c.target == Target::App("netflix".into()))
        .unwrap()
        .rect;
    assert!(frame.draw.iter().any(|draw| matches!(draw,
        Draw::Rim(rect, _, width, ..) if *width > view::FOCUS_RIM && rect.x == netflix.x && rect.y == netflix.y)));
    for control in &before {
        if let Some(other) = frame.controls.iter().find(|c| c.target == control.target) {
            assert_eq!(
                (control.rect.x, control.rect.y),
                (other.rect.x, other.rect.y)
            );
        }
    }
    state.back();
    assert!(state.moving.is_none());
}

#[test]
fn the_grid_stays_unchanged_until_move_is_chosen() {
    let mut state = ready();
    state.open(Page::Home);
    let faces = card_faces(&state);
    // A remote long press opens the menu; the grid behind it is untouched, not in move mode.
    state.focus = Target::App("netflix".into());
    state.hold();
    assert!(state.moving.is_none());
    assert!(!state.animating());
    // Only choosing Move starts move mode.
    assert!(state.activate().is_empty());
    assert_eq!(state.moving.as_deref(), Some("netflix"));
    state.back();
    assert_eq!(card_faces(&state), faces);
    // A pointer hold that has not moved yet changes nothing either; releasing opens the menu.
    let rect = controls(&state)
        .into_iter()
        .find(|c| c.target == Target::App("youtube.leanback.v4".into()))
        .unwrap()
        .rect;
    state.drag_start(
        "youtube.leanback.v4",
        (rect.x + 20., rect.y + 20.),
        rect,
        true,
    );
    state.drag_move((rect.x + 25., rect.y + 22.), &controls(&state));
    assert!(state.moving.is_none());
    assert_eq!(card_faces(&state), faces);
    assert!(state.drag_end().is_empty());
    assert!(state.moving.is_none());
    assert!(matches!(state.overlay, Some(Overlay::Options { .. })));
    // Once it moves past the threshold it is a drag, and only then move mode.
    state.back();
    state.drag_start(
        "youtube.leanback.v4",
        (rect.x + 20., rect.y + 20.),
        rect,
        true,
    );
    state.drag_move((rect.x + 80., rect.y + 20.), &controls(&state));
    assert_eq!(state.moving.as_deref(), Some("youtube.leanback.v4"));
}

#[test]
fn the_location_keyboard_is_one_row_and_suggests_places_as_you_type() {
    use crate::state::{Overlay, TextField};
    let mut state = ready();
    state.focus = Target::WeatherConfigure;
    state.activate();
    assert!(matches!(state.overlay, Some(Overlay::Keyboard)));
    assert_eq!(state.keyboard_field, TextField::Location);
    // One row: every visible key shares a y, and the row scrolls with focus.
    let keys: Vec<_> = controls(&state)
        .into_iter()
        .filter(|c| matches!(c.target, Target::Key(_)))
        .collect();
    assert!(keys.len() > 10 && keys.len() < crate::state::KEYS.len());
    assert!(keys.iter().all(|key| key.rect.y == keys[0].rect.y));
    for _ in 0..30 {
        state.direction(Direction::Right, &controls(&state));
    }
    assert!(matches!(state.focus, Target::Key(index) if index >= 30));
    // Typing pauses, then the place search runs on its own.
    let start = std::time::Instant::now();
    state.text("W");
    assert!(state.suggest_locations(start).is_none());
    assert!(
        state
            .suggest_locations(start + std::time::Duration::from_secs(1))
            .is_none(),
        "One character does not search"
    );
    state.text("enen");
    assert!(state.suggest_locations(start).is_none());
    let search = state.suggest_locations(start + std::time::Duration::from_secs(1));
    assert!(
        matches!(search, Some(crate::weather::Command::Search { ref query, .. }) if query == "Wenen")
    );
    // Results appear beneath the keys; Down reaches them, OK picks one and closes the keyboard.
    let place = crate::weather::Location {
        name: "Wenen".into(),
        region: "Wenen".into(),
        country: "Oostenrijk".into(),
        latitude: 48.2085,
        longitude: 16.3721,
    };
    state.weather.searching = false;
    state.weather.results = vec![place.clone()];
    let rendered = controls(&state);
    let result = rendered
        .iter()
        .find(|c| c.target == Target::WeatherResult(0))
        .unwrap();
    assert!(result.rect.y > keys[0].rect.y);
    state.direction(Direction::Down, &rendered);
    assert_eq!(state.focus, Target::WeatherResult(0));
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert!(state.overlay.is_none());
    assert_eq!(state.preferences.weather_location, Some(place));
}

#[test]
fn dutch_defaults_to_a_24_hour_clock_and_a_lowercase_date_after_it() {
    use crate::i18n::Language;
    let at = time::OffsetDateTime::parse(
        "2026-11-09T23:56:00+01:00",
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    // A Dutch file that never chose a format, including one that saved the old default.
    for saved in [
        json!({"language": "nl"}),
        json!({"language": "nl", "hour_24": false}),
    ] {
        let preferences: Preferences = serde_json::from_value(saved).unwrap();
        assert!(preferences.uses_24_hour());
    }
    // English keeps 12 hours; an earlier saved 24-hour choice stays.
    let english: Preferences = serde_json::from_value(json!({"language": "en"})).unwrap();
    assert!(!english.uses_24_hour());
    let legacy: Preferences =
        serde_json::from_value(json!({"language": "en", "hour_24": true})).unwrap();
    assert!(legacy.uses_24_hour());
    // A format chosen in Settings wins over the language, and survives saving.
    let mut chosen: Preferences = serde_json::from_value(json!({"language": "nl"})).unwrap();
    chosen.set_24_hour(false);
    let reloaded: Preferences =
        serde_json::from_value(serde_json::to_value(&chosen).unwrap()).unwrap();
    assert!(!reloaded.uses_24_hour());
    let mut state = ready();
    state.preferences.language = Language::Nl;
    state.open(Page::Settings);
    state.focus = Target::ClockFormat;
    state.subpage = Some(crate::state::Subpage::Clock);
    state.activate();
    assert_eq!(state.focus, Target::Choice(1), "24-hour is selected");
    // The date after the clock is lowercase in Dutch; starting a line it is sentence case.
    let format = crate::model::DateFormat::Localized;
    assert_eq!(format.format_inline(Language::Nl, at), "maandag 9 november");
    assert_eq!(format.format(Language::Nl, at), "Maandag 9 november");
    assert_eq!(format.format_inline(Language::En, at), "Monday, November 9");
    assert_eq!(
        Language::Nl.clock(at, reloaded.uses_24_hour(), false),
        "11:56 PM"
    );
    assert_eq!(Language::Nl.clock(at, true, false), "23:56");
}

#[test]
fn the_top_bar_keeps_settings_without_the_app_store_and_icons_are_clickable() {
    let mut state = State::new(Preferences::default());
    let mut inventory = demo_inventory();
    inventory
        .apps
        .retain(|app| app.id != crate::state::STORE_ID);
    state.set_inventory(inventory, None);
    assert_eq!(state.top_bar(), [Target::SettingsIcon]);
    let bar: Vec<_> = controls(&state)
        .into_iter()
        .filter(|c| matches!(c.target, Target::StoreIcon | Target::SettingsIcon))
        .collect();
    assert_eq!(bar.len(), 1);
    // Small icons on the clock's line, right-aligned inside the safe area.
    for icon in &bar {
        assert_eq!(icon.rect.w, view::TOP_ICON);
        assert!(icon.rect.x + icon.rect.w <= 1920. - view::SAFE_X);
        assert!((icon.rect.y + icon.rect.h / 2. - (view::CLOCK_Y + 18.)).abs() < 0.5);
    }
    // A pointer click on Settings opens it.
    let settings = bar
        .iter()
        .find(|c| c.target == Target::SettingsIcon)
        .unwrap();
    let (x, y) = settings.rect.center();
    assert_eq!(
        state.hit_test(&controls(&state), x, y),
        Some(Target::SettingsIcon)
    );
    state.pointer_focus(Target::SettingsIcon);
    state.activate();
    assert_eq!(state.page, Page::Settings);
}

#[test]
fn every_settings_list_keeps_its_last_row_inside_the_safe_area_and_fades_at_the_bottom() {
    use crate::renderer::Draw;
    use crate::state::{SETTINGS_LIST_FADE, SETTINGS_SAFE_BOTTOM, Subpage};
    let mut state = ready();
    state.open(Page::Settings);
    state.activate();
    // Walk down General to its last row: every row stays above the fade.
    let mut previous = None;
    while previous.as_ref() != Some(&state.focus) {
        previous = Some(state.focus.clone());
        state.direction(Direction::Down, &controls(&state));
        let rect = controls(&state)
            .into_iter()
            .find(|c| c.target == state.focus)
            .unwrap()
            .rect;
        assert!(
            rect.y + rect.h <= SETTINGS_SAFE_BOTTOM - SETTINGS_LIST_FADE + 0.01,
            "{:?} ends at {}",
            state.focus,
            rect.y + rect.h
        );
    }
    // A list longer than the screen (many hidden apps) scrolls to keep its rows above the fade.
    state.preferences.hidden = state
        .apps()
        .iter()
        .take(14)
        .map(|app| app.id.clone())
        .collect();
    state.focus = Target::Setting(11);
    state.activate();
    assert_eq!(state.subpage, Some(Subpage::HiddenApps));
    // (Down from the last hidden app wraps to the first, so step exactly to the last row.)
    for _ in 1..state.hidden_apps().len() {
        state.direction(Direction::Down, &controls(&state));
        let rect = controls(&state)
            .into_iter()
            .find(|c| c.target == state.focus)
            .unwrap()
            .rect;
        assert!(rect.y + rect.h <= SETTINGS_SAFE_BOTTOM - SETTINGS_LIST_FADE + 0.01);
    }
    assert!(
        state.scroll > 0.,
        "the hidden apps list is longer than the screen"
    );
    // Wheel scrolling stops with the last row above the fade, too.
    state.wheel_scroll(&controls(&state), Some((1000., 600.)), 0, -1000);
    let last = controls(&state)
        .into_iter()
        .filter(|c| matches!(c.target, Target::Choice(_)))
        .map(|c| c.rect.y + c.rect.h)
        .fold(0., f32::max);
    assert!((last - (SETTINGS_SAFE_BOTTOM - SETTINGS_LIST_FADE)).abs() < 0.01);
    // Rows below fade out before the safe-area line instead of being cut at the screen edge.
    assert!(scene(&state).draw.iter().any(|draw| matches!(draw,
        Draw::EdgeFade(Some(fade)) if fade[3] == SETTINGS_SAFE_BOTTOM && fade[2] < fade[3])));
}

#[test]
fn notices_are_a_small_pill_centred_above_the_bottom_safe_area() {
    use crate::renderer::Draw;
    let mut state = ready();
    state.notify("Netflix added to favorites.");
    let rendered = scene(&state);
    let pill = rendered
        .controls
        .iter()
        .find(|c| c.target == Target::DismissNotice)
        .unwrap()
        .rect;
    assert!((pill.x + pill.w / 2. - 960.).abs() < 0.5, "Centred");
    assert_eq!(pill.y + pill.h, 1020., "Its bottom on the safe-area line");
    assert!(pill.h <= 56., "Compact: {}", pill.h);
    assert!(rendered.draw.iter().any(|draw| matches!(draw,
        Draw::Text(text, _, style, ..) if text == "Netflix added to favorites" && style.size == 25.)));
    // A new notice replaces the current one.
    state.notify("Weather unavailable");
    let rendered = scene(&state);
    assert_eq!(
        rendered
            .controls
            .iter()
            .filter(|c| c.target == Target::DismissNotice)
            .count(),
        1
    );
    assert!(!rendered.draw.iter().any(|draw| matches!(draw,
        Draw::Text(text, ..) if text.starts_with("Netflix added"))));
}

#[test]
fn startup_diagnostics_are_written_as_json_next_to_the_preferences() {
    let directory = std::env::temp_dir().join(format!("luma-diagnostics-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("luma-diagnostics.json");
    let facts = json!({"text_rendering": "cpu glyph atlas", "obtained_samples": 4, "density": 1.0});
    crate::diagnostics::write_startup(&path, &facts).unwrap();
    crate::diagnostics::write_startup(&path, &facts).unwrap();
    let read: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(read, facts, "Overwritten, not appended");
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn focused_card_only_brightens_in_place_and_focus_moves_instantly() {
    use crate::renderer::Draw;
    let mut state = ready();
    state.open(Page::Home);
    state.sync_motion();
    let netflix = Target::App("netflix".into());
    assert_eq!(state.focus, netflix);
    let card = |state: &State, target: &Target| {
        controls(state)
            .into_iter()
            .find(|control| control.target == *target)
            .unwrap()
            .rect
    };
    use crate::state::Rect;
    let same = |a: Rect, b: Rect| a.x == b.x && a.y == b.y && a.w == b.w && a.h == b.h;
    let highlight = |state: &State, rect: Rect| {
        let drawn = scene(state).draw;
        let rim = drawn.iter().any(|draw| {
            matches!(draw, Draw::Rim(r, _, width, ..) if same(*r, rect) && *width == view::FOCUS_RIM)
        });
        let overlay = drawn.iter().any(|draw| {
            matches!(draw, Draw::Shape(r, _, _, color)
                if same(*r, rect) && *color == [1., 1., 1., view::FOCUS_OVERLAY])
        });
        (rim, overlay)
    };
    let first = card(&state, &netflix);
    assert_eq!(highlight(&state, first), (true, true));
    // Nothing is drawn larger than the card: no lift or scale.
    assert!(!scene(&state).draw.iter().any(|draw| matches!(draw,
        Draw::Rim(r, ..) if r.w > first.w + 0.01 && (r.center().0 - first.center().0).abs() < 1.)));
    // Moving focus highlights the next card in the very next frame and clears the old one.
    let all = controls(&state);
    state.direction(Direction::Right, &all);
    state.sync_motion();
    let next = state.focus.clone();
    assert!(matches!(next, Target::App(_)) && next != netflix);
    assert_eq!(state.focus_level(&next), 1.);
    assert_eq!(state.focus_level(&netflix), 0.);
    assert_eq!(highlight(&state, card(&state, &next)), (true, true));
    assert_eq!(highlight(&state, first), (false, false));
}

#[test]
fn settings_focus_changes_in_the_first_frame_and_never_moves_afterwards() {
    use crate::renderer::Draw;
    use std::time::Duration;
    let mut state = ready();
    state.open(Page::Settings);
    state.sync_motion();
    let same_scene = |a: &[Draw], b: &[Draw]| {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| crate::damage::same(a, b))
    };
    // The opaque white surface of the focused control, and its control rect.
    let focused = |state: &State| {
        let scene = scene(state);
        let control = scene
            .controls
            .iter()
            .find(|control| control.target == state.focus)
            .map(|control| control.rect)
            .unwrap();
        let surfaces: Vec<_> = scene
            .draw
            .iter()
            .filter_map(|draw| match draw {
                Draw::RoundedGradient(rect, _, top, _) if *top == [1., 1., 1., 1.] => Some(*rect),
                _ => None,
            })
            .collect();
        (control, surfaces, scene.draw)
    };
    // Sidebar, into General's rows, and back: no list scrolls along this path.
    for direction in [
        Direction::Down,
        Direction::Up,
        Direction::Right,
        Direction::Down,
        Direction::Down,
        Direction::Up,
        Direction::Left,
    ] {
        let controls = controls(&state);
        state.direction(direction, &controls);
        state.sync_motion();
        let (control, surfaces, first) = focused(&state);
        // The first frame already shows the focused control fully, in place (no grow).
        assert_eq!(
            state.focus_level(&state.focus.clone()),
            1.,
            "{:?}",
            state.focus
        );
        assert!(
            surfaces.iter().any(|rect| rect.x == control.x
                && rect.y == control.y
                && rect.w == control.w
                && rect.h == control.h),
            "{:?}: no full white surface at its rect",
            state.focus
        );
        // Settling changes nothing.
        state.advance_motion(Duration::from_millis(600));
        let (_, _, settled) = focused(&state);
        assert!(
            same_scene(&first, &settled),
            "{:?} moved while settling",
            state.focus
        );
    }
    // Holding OK keeps the focused look in Settings; releasing toggles the value at once.
    state.focus = Target::Setting(0);
    state.sync_motion();
    state.pressed = Some(Target::Setting(0));
    assert_eq!(state.display_level(&Target::Setting(0)), 1.);
    state.pressed = None;
    let on = state.preferences.language.t("stateOn").to_owned();
    let count_on = |state: &State| {
        scene(state)
            .draw
            .iter()
            .filter(|draw| matches!(draw, Draw::Text(text, ..) if *text == on))
            .count()
    };
    let before = count_on(&state);
    // App names, on by default, turns off: the very next scene shows the new value.
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert!(!state.preferences.show_app_names);
    assert_eq!(count_on(&state) + 1, before);
}

#[test]
fn auto_inputs_follow_hdmi_plug_in_not_the_eim_connected_flag() {
    // The B4's reply with every source in standby: eim still says `connected` for ports with a
    // cable, but `hdmiPlugIn` (the +5 V cable detect) is false. Auto follows `hdmiPlugIn`
    // (upstream #27), so the top bar shows no input glyph, in Dark and Light alike.
    let reply = json!([
        {"appId":"com.webos.app.hdmi1","id":"HDMI_1","port":1,"connected":true,
         "hdmiPlugIn":false,"hdmiSignalExist":false},
        {"appId":"com.webos.app.hdmi2","id":"HDMI_2","port":2,"connected":true,
         "hdmiPlugIn":false,"hdmiSignalExist":false},
        {"appId":"com.webos.app.hdmi3","id":"HDMI_3","port":3,"connected":false,
         "hdmiPlugIn":false,"hdmiSignalExist":false},
        {"appId":"com.webos.app.hdmi4","id":"HDMI_4","port":4,"connected":false,
         "hdmiPlugIn":false,"hdmiSignalExist":false}
    ]);
    let mut state = ready();
    state.preferences.show_inputs = true;
    state.preferences.input_modes.clear();
    state.set_inputs(normalize_inputs(&reply).unwrap());
    assert_eq!(state.inventory.inputs.len(), 4);
    assert!(state.home_inputs().is_empty());
    assert_eq!(state.top_bar(), [Target::StoreIcon, Target::SettingsIcon]);
    // A source that drives the cable (as during the round-8 capture) shows up in Auto…
    let mut powered = reply.clone();
    powered[0]["hdmiPlugIn"] = json!(true);
    state.set_inputs(normalize_inputs(&powered).unwrap());
    assert_eq!(
        state.top_bar()[0],
        Target::Input("com.webos.app.hdmi1".into())
    );
    // …and "On" keeps an input in the bar whatever the cable reports.
    state.set_inputs(normalize_inputs(&reply).unwrap());
    state
        .preferences
        .input_modes
        .insert("com.webos.app.hdmi2".into(), InputMode::On);
    assert_eq!(
        state.top_bar()[0],
        Target::Input("com.webos.app.hdmi2".into())
    );
    // In Light appearance the glyph is drawn in dark ink, never white on the light scrim
    // (dimmed, as in Dark, while its cable reports nothing).
    state.open(Page::Home);
    state.preferences.appearance = crate::model::Appearance::Light;
    let glyph = scene(&state)
        .draw
        .iter()
        .find_map(|draw| match draw {
            crate::renderer::Draw::TintedImage(path, _, _, _, color)
                if path.contains("ui-icons/input-") =>
            {
                Some(*color)
            }
            _ => None,
        })
        .expect("the input glyph is drawn");
    assert!(glyph[0] < 0.3 && glyph[3] >= 0.38, "{glyph:?}");
}

#[test]
fn page_switches_are_instant_and_every_frame_is_fully_composed() {
    use crate::renderer::Draw;
    use std::time::Duration;
    // Draws outside `skip` (the Home top-bar icon that focus returns to, which keeps its
    // short lift); everything else must already be final in the first frame.
    let steady = |state: &mut State| {
        state.advance_motion(Duration::from_millis(800));
        scene(state).draw
    };
    let compare = |first: &[Draw], settled: &[Draw], skip: Option<crate::state::Rect>| {
        let keep = |draws: &[Draw]| -> Vec<Draw> {
            draws
                .iter()
                .filter(|draw| {
                    skip.is_none_or(|area| {
                        crate::damage::bounds(draw).is_none_or(|bounds| {
                            !crate::damage::intersects(bounds, area.inset(-80.))
                        })
                    })
                })
                .cloned()
                .collect()
        };
        let (a, b) = (keep(first), keep(settled));
        a.len() == b.len() && a.iter().zip(&b).all(|(a, b)| crate::damage::same(a, b))
    };
    let mut state = ready();
    state.open(Page::Home);
    state.sync_motion();
    let home = steady(&mut state);
    // Home → Settings from its icon: the very first frame is the finished Settings page.
    state.focus = Target::SettingsIcon;
    state.sync_motion();
    assert!(state.activate().is_empty());
    assert_eq!(state.page, Page::Settings);
    state.sync_motion();
    state.tick();
    let first = scene(&state).draw;
    assert!(!compare(&first, &home, None), "Settings is drawn at once");
    let settings = steady(&mut state);
    assert!(
        compare(&first, &settings, None),
        "no fade or partial frame into Settings"
    );
    // No frame shows the bare wallpaper: the first Settings frame has its glass and text.
    assert!(first.iter().any(|draw| matches!(draw, Draw::Text(..))));
    // Back → Home: the first frame is Home with its dim, shelf and grid at full opacity.
    state.back();
    assert_eq!(state.page, Page::Home);
    state.sync_motion();
    state.tick();
    let back = scene(&state);
    let icon = back
        .controls
        .iter()
        .find(|control| control.target == Target::SettingsIcon)
        .map(|control| control.rect);
    let settled = steady(&mut state);
    assert!(compare(&back.draw, &settled, icon), "no fade back to Home");
    assert!(back.draw.iter().any(|draw| matches!(draw,
        Draw::Shape(rect, 0., 0., color) if rect.w >= 1920. && color[3] == view::HOME_DIM)));
}

#[test]
fn overlays_are_complete_in_their_first_frame_and_the_hold_preview_changes_nothing() {
    use crate::renderer::Draw;
    use std::time::Duration;
    let same = |a: &[Draw], b: &[Draw]| {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| crate::damage::same(a, b))
    };
    let first_and_settled = |state: &mut State| {
        state.sync_motion();
        state.tick();
        let first = scene(state).draw;
        state.advance_motion(Duration::from_millis(800));
        (first, scene(state).draw)
    };
    let mut state = ready();
    state.open(Page::Home);
    state.sync_motion();
    let plain = state
        .grid()
        .into_iter()
        .find(|target| matches!(target, Target::App(_)))
        .unwrap();
    // While OK is held, the menu a long press opens can be built (its labels rasterised)
    // without changing anything; every app's long press opens that menu.
    state.focus = plain.clone();
    state.dirty = false;
    let menu = state
        .with_hold_preview(scene)
        .expect("an app's long press opens its menu");
    assert!(menu.modal.is_some());
    assert!(state.overlay.is_none() && state.focus == plain && !state.dirty);
    state.focus = Target::SettingsIcon;
    assert!(state.with_hold_preview(scene).is_none());
    // The long press opens the menu complete in its first frame: no fade, no lift easing.
    state.focus = plain.clone();
    state.hold();
    assert!(matches!(state.overlay, Some(Overlay::Options { .. })));
    let (first, settled) = first_and_settled(&mut state);
    assert!(same(&first, &settled), "the menu fades or moves in");
    // Everything the shown frame writes was prepared, in the same styles (glyph keys depend on
    // text, size and weight only, not on the focus colours).
    let texts = |draws: &[Draw]| -> Vec<(String, u32, u16)> {
        draws
            .iter()
            .filter_map(|draw| match draw {
                Draw::Text(text, _, style, ..) => {
                    Some((text.clone(), style.size.to_bits(), style.weight))
                }
                _ => None,
            })
            .collect()
    };
    let prepared = texts(&menu.draw);
    for text in texts(&first) {
        assert!(prepared.contains(&text), "{text:?} was not prepared");
    }
    // Its delete alert, and closing both, are instant too.
    state.focus = Target::Option(2);
    state.activate();
    assert!(matches!(state.overlay, Some(Overlay::Delete { .. })));
    let (first, settled) = first_and_settled(&mut state);
    assert!(same(&first, &settled), "the alert fades or moves in");
    state.back();
    state.back();
    assert!(state.overlay.is_none());
    let (first, settled) = first_and_settled(&mut state);
    assert!(same(&first, &settled), "closing fades");
    assert!(!state.animating());
}

#[test]
fn the_first_run_migrates_favourites_into_one_saved_order_and_appends_new_apps() {
    let mut state = State::new(
        serde_json::from_value(json!({"favorites": ["youtube.leanback.v4", "netflix"]})).unwrap(),
    );
    let mut inventory = demo_inventory();
    let late = inventory.apps.pop().unwrap();
    // The first inventory stores the order once: former favourites first, then the rest by
    // title.
    assert!(state.set_inventory(inventory.clone(), None));
    let order = state.preferences.app_order.clone().unwrap();
    assert_eq!(&order[..2], ["youtube.leanback.v4", "netflix"]);
    let titles: Vec<String> = order[2..]
        .iter()
        .map(|id| {
            let app = inventory.apps.iter().find(|app| &app.id == id).unwrap();
            app.title.to_lowercase()
        })
        .collect();
    let mut sorted = titles.clone();
    sorted.sort();
    assert_eq!(titles, sorted);
    assert_eq!(order.len(), inventory.apps.len());
    // Saved with the order; the former favourites stay as they were.
    let saved = serde_json::to_value(&state.preferences).unwrap();
    assert_eq!(saved["app_order"], json!(order));
    assert_eq!(
        saved["favorites"],
        json!(["youtube.leanback.v4", "netflix"])
    );
    // The same inventory changes nothing; a newly installed app joins the end.
    assert!(!state.set_inventory(inventory.clone(), None));
    inventory.apps.push(late.clone());
    assert!(state.set_inventory(inventory.clone(), None));
    let order = state.preferences.app_order.clone().unwrap();
    assert_eq!(order.last(), Some(&late.id));
    // A saved order is never replaced by the favourites again.
    let reloaded: Preferences = serde_json::from_value(saved).unwrap();
    let mut again = State::new(reloaded);
    again.set_inventory(inventory, None);
    assert_eq!(
        &again.preferences.app_order.unwrap()[..2],
        ["youtube.leanback.v4", "netflix"]
    );
}

#[test]
fn a_moving_app_steps_one_slot_in_all_four_directions_and_wraps_across_rows() {
    let mut state = ready();
    state.open(Page::Home);
    let index = |state: &State, id: &str| state.apps().iter().position(|app| app.id == id).unwrap();
    let id = state.apps()[7].id.clone();
    start_moving(&mut state, &id);
    let go = |state: &mut State, direction| state.direction(direction, &controls(state));
    // Six columns: Left/Right step through the order, Up/Down move a whole row.
    assert_eq!(go(&mut state, Direction::Left), vec![Effect::Save]);
    assert_eq!(index(&state, &id), 6);
    // Left from the start of a row continues at the end of the row above.
    go(&mut state, Direction::Left);
    assert_eq!(index(&state, &id), 5);
    go(&mut state, Direction::Right);
    go(&mut state, Direction::Right);
    assert_eq!(index(&state, &id), 7);
    go(&mut state, Direction::Up);
    assert_eq!(index(&state, &id), 1);
    // The first row has nothing above; the move does nothing and saves nothing.
    assert!(go(&mut state, Direction::Up).is_empty());
    assert_eq!(index(&state, &id), 1);
    go(&mut state, Direction::Down);
    go(&mut state, Direction::Down);
    assert_eq!(index(&state, &id), 13);
    // The others shift by one slot; focus stays on the moving app; Back keeps the order.
    assert_eq!(state.focus, Target::App(id.clone()));
    assert!(state.back().is_empty());
    assert!(state.moving.is_none());
    assert_eq!(index(&state, &id), 13);
    let last = state.apps().len() - 1;
    let end = state.apps()[last].id.clone();
    start_moving(&mut state, &end);
    assert!(go(&mut state, Direction::Right).is_empty());
    assert!(go(&mut state, Direction::Down).is_empty());
    assert_eq!(index(&state, &end), last);
}

#[test]
fn the_pointer_drags_a_card_to_a_new_slot_and_a_still_hold_opens_its_options() {
    use crate::renderer::Draw;
    let mut state = ready();
    state.open(Page::Home);
    let card = |state: &State, id: &str| {
        controls(state)
            .into_iter()
            .find(|c| c.target == Target::App(id.into()))
            .unwrap()
            .rect
    };
    let order =
        |state: &State| -> Vec<String> { state.apps().iter().map(|app| app.id.clone()).collect() };
    let before = order(&state);
    let (dragged, onto) = (before[0].clone(), before[8].clone());
    // Holding the pointer button on a card picks it up; moving makes it a drag (move mode).
    let from = card(&state, &dragged);
    let grab = (from.x + 40., from.y + 30.);
    state.drag_start(&dragged, grab, from, true);
    assert!(
        state.moving.is_none(),
        "nothing changes until it is dragged"
    );
    // A few pixels are not a drag yet.
    state.drag_move((grab.0 + 5., grab.1 + 3.), &controls(&state));
    assert!(!state.drag.as_ref().unwrap().active);
    // Over another card: the card follows the pointer and that slot is the drop target.
    let target = card(&state, &onto);
    let over = (target.x + target.w / 2., target.y + target.h / 2.);
    let slots = controls(&state);
    state.drag_move(over, &slots);
    let drag = state.drag.clone().unwrap();
    assert!(drag.active);
    assert_eq!(drag.target.as_deref(), Some(onto.as_str()));
    let frame = scene(&state);
    // Every card's hit region stays at its slot while dragging.
    for control in slots.iter().filter(|c| matches!(c.target, Target::App(_))) {
        let now = frame
            .controls
            .iter()
            .find(|c| c.target == control.target)
            .unwrap();
        assert_eq!((now.rect.x, now.rect.y), (control.rect.x, control.rect.y));
    }
    // The drop slot is outlined, and the dragged card is drawn last, where the pointer holds it.
    assert!(frame.draw.iter().any(|draw| matches!(draw,
        Draw::Shape(rect, _, border, _) if *border == 4. && rect.x == target.x && rect.y == target.y)));
    let held = (over.0 - (grab.0 - from.x), over.1 - (grab.1 - from.y));
    let face = frame
        .draw
        .iter()
        .rposition(|draw| {
            matches!(draw,
            Draw::Image(_, rect, ..) | Draw::AppImage(_, rect, ..)
                if (rect.x - held.0).abs() < 0.01 && (rect.y - held.1).abs() < 0.01
                    && rect.w == from.w)
        })
        .expect("the dragged card is drawn at the pointer");
    let last_card_face = frame
        .draw
        .iter()
        .rposition(|draw| {
            matches!(draw,
            Draw::Image(_, rect, ..) | Draw::AppImage(_, rect, ..) if rect.w == from.w)
        })
        .unwrap();
    assert_eq!(face, last_card_face, "drawn over the other cards");
    // Releasing drops it into that slot; the others shift and move mode ends.
    assert_eq!(state.drag_end(), vec![Effect::Save]);
    assert!(state.moving.is_none() && state.drag.is_none());
    let after = order(&state);
    assert_eq!(after.iter().position(|id| id == &dragged), Some(8));
    assert_eq!(after[0], before[1]);
    // A hold that never moves opens the card's options, as a long press does.
    let still = card(&state, &before[2]);
    state.drag_start(&before[2], (still.x + 10., still.y + 10.), still, true);
    assert!(state.drag_end().is_empty());
    assert!(matches!(&state.overlay, Some(Overlay::Options { id, .. }) if id == &before[2]));
    state.back();
    // In move mode (from the menu), picking the card up without moving keeps move mode.
    start_moving(&mut state, &before[3]);
    let rect = card(&state, &before[3]);
    state.drag_start(&before[3], (rect.x + 10., rect.y + 10.), rect, false);
    assert!(state.drag_end().is_empty());
    assert_eq!(state.moving.as_deref(), Some(before[3].as_str()));
}

#[test]
fn the_screensaver_clock_fades_out_and_in_around_each_jump_and_never_brightens() {
    use crate::renderer::Draw;
    use crate::state::{SCREENSAVER_FADE, SCREENSAVER_STEP, screensaver_opacity};
    use std::time::{Duration, Instant};
    let at = |ms: u64| screensaver_opacity(Duration::from_millis(ms));
    let step = SCREENSAVER_STEP.as_millis() as u64;
    let fade = SCREENSAVER_FADE.as_millis() as u64;
    assert!((300..=400).contains(&fade), "a subtle 300–400 ms fade");
    // Fully shown between fades; out before the jump, in after it; eased at both ends.
    assert_eq!(at(fade), 1.);
    assert_eq!(at(step / 2), 1.);
    assert_eq!(at(step - fade), 1.);
    assert_eq!(at(step), 0.);
    assert!((at(step + fade / 2) - 0.5).abs() < 0.01);
    assert!(at(step + fade / 10) < 0.1, "ease-in: slow at the start");
    assert!(
        at(step + fade - fade / 10) > 0.9,
        "ease-out: slow at the end"
    );
    let mut previous = 0.;
    for ms in (step..=step + fade).step_by(10) {
        let alpha = at(ms);
        assert!(alpha >= previous && alpha <= 1., "{ms} ms: {alpha}");
        previous = alpha;
        // Fading out is the mirror image of fading in.
        assert!((at(2 * step - (ms - step)) - alpha).abs() < 1e-4);
    }
    // In the scene: only the text's opacity changes, never above its resting grey, on black.
    let mut state = ready();
    state.start_screensaver();
    let (_, seed) = state.screensaver.unwrap();
    let colors = |state: &State| -> Vec<[f32; 4]> {
        let draws = scene(state).draw;
        assert!(matches!(&draws[0], Draw::Shape(_, _, _, color) if *color == [0., 0., 0., 1.]));
        draws
            .iter()
            .filter_map(|draw| match draw {
                Draw::Text(_, _, _, color, _) => Some(*color),
                _ => None,
            })
            .collect()
    };
    state.screensaver = Some((Instant::now() - Duration::from_millis(step / 2), seed));
    let resting = colors(&state);
    assert_eq!(resting.len(), 2);
    for ms in [step - fade / 2, step + fade / 4, step + fade / 2] {
        state.screensaver = Some((Instant::now() - Duration::from_millis(ms), seed));
        let alpha = state.screensaver_alpha();
        assert!(alpha > 0. && alpha < 1.);
        for (color, rest) in colors(&state).iter().zip(&resting) {
            assert_eq!(color[..3], rest[..3], "same grey");
            assert!(color[3] <= rest[3] + 1e-6 && (color[3] - rest[3] * alpha).abs() < 0.02);
        }
        assert!(state.animating(), "frames are drawn while it fades");
    }
    // Between fades the loop sleeps until the next fade-out begins.
    state.screensaver = Some((Instant::now() - Duration::from_millis(1000), seed));
    assert!(!state.animating());
    let wake = state.screensaver_wake().unwrap();
    assert!(wake <= SCREENSAVER_STEP - SCREENSAVER_FADE - Duration::from_millis(990));
}

#[test]
fn removed_settings_still_load_from_old_preference_files_and_their_rows_are_gone() {
    // Text size, Night wallpaper, Rotation, Gradient and Reduced motion were removed; files that
    // still carry them load with everything else intact and drop them when saved.
    let old: Preferences = serde_json::from_value(json!({
        "larger_text": true,
        "night_background": "forest",
        "wallpaper_cycle": "hourly",
        "wallpaper_changed": 1_700_000_000u64,
        "wallpaper_gradient": false,
        "reduced_motion": true,
        "background": "summit",
        "greeting_name": "Ana",
        "appearance": "light"
    }))
    .unwrap();
    assert_eq!(old.background, "summit");
    assert_eq!(old.greeting_name, "Ana");
    assert_eq!(old.appearance, crate::model::Appearance::Light);
    let saved = serde_json::to_value(&old).unwrap();
    let saved = saved.as_object().unwrap();
    for key in [
        "larger_text",
        "night_background",
        "wallpaper_cycle",
        "wallpaper_changed",
        "wallpaper_gradient",
        "reduced_motion",
    ] {
        assert!(!saved.contains_key(key), "{key}");
    }
    // General keeps App names, Clock, Language, Hidden apps, Appearance and Screensaver.
    let mut state = ready();
    state.open(Page::Settings);
    let rows: Vec<usize> = controls(&state)
        .into_iter()
        .filter_map(|c| match c.target {
            Target::Setting(index) => Some(index),
            _ => None,
        })
        .collect();
    assert_eq!(rows, [0, 1, 4, 11, 14, 12]);
}
