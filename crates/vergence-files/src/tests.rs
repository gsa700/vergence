//! FileView's window's tests (main.rs's `mod tests`).

use super::*;
use crate::drives::{df_free, parse_df_free};
use crate::tab::as_trashed;
use std::path::Path;

fn shared() -> SharedRef {
    lone_shared()
}

#[test]
fn the_trash_lists_names_they_had_and_where_from() {
    let d = std::env::temp_dir().join(format!("vergence-files-trash-list-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let (files_dir, info) = (d.join("files"), d.join("info"));
    std::fs::create_dir_all(&files_dir).unwrap();
    std::fs::create_dir_all(&info).unwrap();
    // Two notes.txt thrown away: the second has a name of its own in the Trash.
    std::fs::write(files_dir.join("notes.txt"), "one").unwrap();
    std::fs::write(files_dir.join("notes 2.txt"), "two").unwrap();
    std::fs::write(
        info.join("notes.txt.trashinfo"),
        "[Trash Info]\nPath=/home/u/Documents/notes.txt\nDeletionDate=2026-11-17T10:00:00\n",
    )
    .unwrap();
    std::fs::write(
        info.join("notes 2.txt.trashinfo"),
        "[Trash Info]\nPath=/home/u/src/notes.txt\nDeletionDate=2026-11-18T10:00:00\n",
    )
    .unwrap();
    // One with no info (put there by hand) keeps its own name.
    std::fs::write(files_dir.join("stray"), "").unwrap();
    let mut v = files::read_folder(&files_dir).unwrap();
    let from = as_trashed(&mut v, &d);
    let by = |p: &str| v.iter().find(|e| e.path == files_dir.join(p)).unwrap();
    assert_eq!(by("notes.txt").name, "notes.txt");
    assert_eq!(by("notes 2.txt").name, "notes.txt", "shown by the name it had");
    assert_eq!(
        from.get(&files_dir.join("notes 2.txt")).map(PathBuf::as_path),
        Some(Path::new("/home/u/src/notes.txt"))
    );
    assert_eq!(
        by("notes 2.txt").changed,
        std::fs::metadata(info.join("notes 2.txt.trashinfo")).and_then(|m| m.modified()).ok(),
        "deleted when its info was written"
    );
    assert_eq!(by("stray").name, "stray");
    assert!(!from.contains_key(&files_dir.join("stray")));
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_tab_goes_back_forward_and_up() {
    let dir = std::env::temp_dir().join(format!("vergence-files-{}", std::process::id()));
    let sub = dir.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(dir.join("a.txt"), "x").unwrap();
    let mut v = Viewer::new(shared());
    v.open_paths(std::slice::from_ref(&dir));
    assert_eq!(v.visible().iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["sub", "a.txt"], "folders first");
    v.go(Showing::Folder(sub.clone()));
    v.up();
    let t = v.front_tab().unwrap();
    assert_eq!(t.showing, Showing::Folder(dir.clone()));
    assert_eq!(t.chosen, vec![sub.clone()], "the folder come up from is chosen");
    v.back();
    assert_eq!(v.front_tab().unwrap().showing, Showing::Folder(sub.clone()));
    v.forward();
    assert_eq!(v.front_tab().unwrap().showing, Showing::Folder(dir.clone()));
    // A file given opens its folder with it chosen.
    let mut v = Viewer::new(shared());
    v.open_paths(&[dir.join("a.txt")]);
    assert_eq!(v.front_tab().unwrap().chosen, vec![dir.join("a.txt")]);
    // Files milestone 3: more folders handed over open in tabs of this
    // window, the newest in front; nothing given, a window of its own.
    v.open_paths(std::slice::from_ref(&sub));
    assert_eq!(v.tabs.len(), 2);
    assert_eq!(v.front_tab().unwrap().showing, Showing::Folder(sub.clone()));
    assert!(v.pending.is_empty());
    v.open_paths(&[]);
    assert_eq!((v.tabs.len(), v.pending.len()), (2, 1));
    v.narrow = "zzz".into();
    assert!(v.visible().is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn free_space_is_read_from_df() {
    let text = "Filesystem     1024-blocks      Used Available Capacity Mounted on\n/dev/nvme0n1p2   479596204 201234567 254000000      45% /\n";
    assert_eq!(parse_df_free(text), Some(254_000_000 * 1024));
    assert_eq!(parse_df_free("only a heading\n"), None);
    // The real thing, on whatever system this is.
    assert!(df_free(&std::env::temp_dir()).is_some_and(|n| n > 0));
}

/// Back and Forward bring each folder back where it was scrolled; a
/// folder new to the tab starts at its top.
#[test]
fn folders_come_back_at_their_scroll_places() {
    let dir = std::env::temp_dir().join(format!("vergence-files-places-{}", std::process::id()));
    let (a, b) = (dir.join("a"), dir.join("b"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let mut v = Viewer::new(shared());
    v.open_paths(std::slice::from_ref(&a));
    v.front_mut().unwrap().scroll = 300.0;
    v.go(Showing::Folder(b.clone()));
    assert_eq!(v.front_mut().unwrap().want.take(), Some(0.0), "a new folder: the top");
    v.front_mut().unwrap().scroll = 75.0;
    v.back();
    assert_eq!(v.front_mut().unwrap().want.take(), Some(300.0));
    v.forward();
    assert_eq!(v.front_mut().unwrap().want.take(), Some(75.0));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Rename in place (step 5): the one chosen item clicked again, then a
/// pause, makes its name a field; a double click opens it instead.
#[test]
fn a_second_click_and_a_pause_renames_a_double_click_opens() {
    let dir = std::env::temp_dir().join(format!("vergence-files-ren-{}", std::process::id()));
    let sub = dir.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let mut v = Viewer::new(shared());
    v.open_paths(std::slice::from_ref(&dir));
    let mut w = Window::default();
    // A double click on the chosen folder: it opens, nothing is renamed.
    v.update(Msg::Click(0), &mut w);
    std::thread::sleep(v.double_gap() + std::time::Duration::from_millis(50));
    v.update(Msg::Click(0), &mut w);
    v.update(Msg::Click(0), &mut w);
    assert_eq!(v.front_tab().unwrap().showing, Showing::Folder(sub.clone()), "opened");
    assert!(v.rename_after.is_none());
    std::thread::sleep(v.double_gap() + std::time::Duration::from_millis(80));
    assert!(!v.timer(RENAME_TIMER, &mut w) && v.naming.is_none(), "no rename after a double click");
    // Back up: a click, a pause, a click, a pause: its name a field in its row.
    v.back();
    v.update(Msg::Click(0), &mut w);
    std::thread::sleep(v.double_gap() + std::time::Duration::from_millis(50));
    v.update(Msg::Click(0), &mut w);
    std::thread::sleep(v.double_gap() + std::time::Duration::from_millis(80));
    assert!(v.timer(RENAME_TIMER, &mut w));
    assert_eq!(v.renaming_row(), Some(0));
    // A click on another place keeps the name typed.
    v.update(Msg::NameTyped("renamed".into()), &mut w);
    v.update(Msg::Run(cmd::RELOAD), &mut w);
    assert!(dir.join("renamed").is_dir() && v.naming.is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Tabs: each keeps its folder and choice; the next and previous go
/// round; a tab taken out leaves the next one in front, and a tab from
/// another window takes this window's folder sizes.
#[test]
fn tabs_keep_their_own_folders() {
    let dir = std::env::temp_dir().join(format!("vergence-files-tabs-{}", std::process::id()));
    let (a, b) = (dir.join("a"), dir.join("b"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let mut v = Viewer::new(shared());
    v.open_paths(&[a.clone(), b.clone()]);
    let ids: Vec<u32> = v.tabs.iter().map(|t| t.id).collect();
    assert_eq!(v.front, ids[1], "the last handed over in front");
    v.narrow = "x".into();
    v.front = ids[0];
    v.tab_changed();
    assert!(v.narrow.is_empty(), "the narrowing is the window's: it starts again");
    assert_eq!(v.front_tab().unwrap().showing, Showing::Folder(a.clone()));
    let t = v.remove_tab(ids[0]).unwrap();
    assert_eq!(v.front, ids[1]);
    let mut other = Viewer::new(shared());
    other.take_tab(t, 0);
    assert!(Arc::ptr_eq(&other.tabs[0].sizes, &other.sizes));
    assert_eq!(other.front_tab().unwrap().showing, Showing::Folder(a.clone()));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_folder_read_again_keeps_its_counted_size() {
    let dir = std::env::temp_dir().join(format!("vergence-files-s-{}", std::process::id()));
    let (big, small) = (dir.join("big"), dir.join("small"));
    std::fs::create_dir_all(&big).unwrap();
    std::fs::create_dir_all(&small).unwrap();
    let mut v = Viewer::new(shared());
    v.open_paths(std::slice::from_ref(&dir));
    let when = |p: &Path| std::fs::metadata(p).unwrap().modified().ok();
    v.sizes.lock().unwrap().insert(big.clone(), (when(&big), Some(9_000)));
    v.sizes.lock().unwrap().insert(small.clone(), (when(&small), Some(10)));
    v.sort = (files::SortBy::Size, false);
    let names = |v: &Viewer| v.visible().iter().map(|e| e.name.clone()).collect::<Vec<_>>();
    assert!(v.front_mut().unwrap().put_sizes(), "counted sizes go into the entries");
    assert_eq!(names(&v), vec!["big", "small"], "largest first");
    // Read again (a change in the folder): the sizes are there at once.
    v.front_mut().unwrap().read();
    assert_eq!(names(&v), vec!["big", "small"]);
    assert!(!v.front_mut().unwrap().put_sizes(), "nothing more to put");
    // A folder changed since it was counted is not given the old size.
    std::fs::write(small.join("x"), "x").unwrap();
    v.front_mut().unwrap().read();
    let e = v.front_tab().unwrap().entries.iter().find(|e| e.name == "small").unwrap().size;
    assert_ne!(e, 10);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_arrows_and_clicks_choose() {
    let dir = std::env::temp_dir().join(format!("vergence-files-c-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for n in ["1.txt", "2.txt", "3.txt"] {
        std::fs::write(dir.join(n), "x").unwrap();
    }
    let mut v = Viewer::new(shared());
    v.open_paths(std::slice::from_ref(&dir));
    v.move_cursor(1);
    v.move_cursor(1);
    assert_eq!(v.front_tab().unwrap().chosen, vec![dir.join("2.txt")]);
    v.mods.shift = true;
    v.click(2);
    assert_eq!(v.front_tab().unwrap().chosen.len(), 2, "Shift+click: a run");
    v.mods = Modifiers { ctrl: true, ..Modifiers::default() };
    v.click(0);
    assert_eq!(v.front_tab().unwrap().chosen.len(), 3, "Ctrl+click adds");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_drop_on_a_sidebar_place_goes_into_it_and_between_rows_is_a_shortcut() {
    use crate::sidebar::{Drop, HEAD_H, Item, Line, PAD, ROW_H, Section, drop_at};
    let item = |name: &str, path: &str, file: bool| Item {
        name: name.into(),
        path: Some(PathBuf::from(path)),
        file,
        place: None,
    };
    let lines = vec![
        Line::Head(Section::Favorites),
        Line::Item(Section::Favorites, item("Home", "/home/dave", false)),
        Line::Item(Section::Favorites, item("Downloads", "/home/dave/Downloads", false)),
        Line::Item(Section::Favorites, item("notes.txt", "/home/dave/notes.txt", true)),
        Line::Head(Section::Locations),
        Line::Item(Section::Locations, item("Trash", "/home/dave/.local/share/Trash/files", false)),
    ];
    let row = |i: usize, share: f32| {
        let top: f32 =
            PAD + lines[..i].iter().map(|l| if matches!(l, Line::Head(_)) { HEAD_H } else { ROW_H }).sum::<f32>();
        let h = if matches!(lines[i], Line::Head(_)) { HEAD_H } else { ROW_H };
        top + h * share
    };
    // The middle of "Downloads": into it (Dave: it made a shortcut).
    assert_eq!(drop_at(&lines, row(2, 0.5)), Drop::Into(2));
    // Its top and bottom quarters: a shortcut before it, or after it.
    assert_eq!(drop_at(&lines, row(2, 0.1)), Drop::Between(1));
    assert_eq!(drop_at(&lines, row(2, 0.9)), Drop::Between(2));
    // The heading: first; a file's row: before or after it, never into it.
    assert_eq!(drop_at(&lines, row(0, 0.5)), Drop::Between(0));
    assert_eq!(drop_at(&lines, row(3, 0.4)), Drop::Between(2));
    // A place under Locations: into it wherever on its row.
    assert_eq!(drop_at(&lines, row(5, 0.1)), Drop::Into(5));
    assert_eq!(drop_at(&lines, row(5, 0.9)), Drop::Into(5));
    assert_eq!(drop_at(&lines, 10_000.0), Drop::Nothing);
}

#[test]
fn a_drop_that_would_do_nothing_or_harm_is_refused_before_it_lands() {
    use crate::dnd::pointless;
    let p = |s: &str| PathBuf::from(s);
    let picked = [p("/m/Pictures/a.jpg"), p("/m/Pictures/b.jpg")];
    // Into the folder they are in: nothing would happen.
    assert!(pointless(&picked, Path::new("/m/Pictures")));
    // Into a folder beside them: a move.
    assert!(!pointless(&picked, Path::new("/m/Pictures/Old")));
    // A folder onto itself, or into a folder inside it.
    let folder = [p("/m/Pictures")];
    assert!(pointless(&folder, Path::new("/m/Pictures")));
    assert!(pointless(&folder, Path::new("/m/Pictures/Old")));
    assert!(!pointless(&folder, Path::new("/m/Downloads")));
    // A drag from elsewhere, not known before it lands: not refused.
    assert!(!pointless(&[], Path::new("/m/Pictures")));
}
