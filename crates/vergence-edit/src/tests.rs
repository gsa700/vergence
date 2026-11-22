//! Edit's window's tests (main.rs's `mod tests`).

use super::*;
use crate::save::suggested_name;

#[test]
fn prose_is_told_from_code_by_its_name() {
    use std::path::Path;
    assert!(is_prose(None), "untitled");
    for p in ["notes.md", "README", "Vergence-wishlist-2", "a.TXT", "doc.rst"] {
        assert!(is_prose(Some(Path::new(p))), "{p}");
    }
    for p in ["main.rs", "index.html", "app.py", "page.php", "desktop.toml", ".zshrc"] {
        assert!(!is_prose(Some(Path::new(p))), "{p}");
    }
}

fn shared() -> SharedRef {
    Rc::new(RefCell::new(Shared {
        windows: Vec::new(),
        transfers: Vec::new(),
        inbox: Arc::new(Mutex::new(Vec::new())),
        listener: Arc::new(Mutex::new(None)),
        next_tab: 0,
        snaps: Vec::new(),
        pending: Vec::new(),
    }))
}

#[test]
fn untitled_documents_are_numbered_and_a_file_replaces_an_empty_one() {
    let mut w = EditWindow::new(shared(), Vec::new());
    assert_eq!(w.tabs.len(), 1);
    assert_eq!(w.tabs[0].name(), "Untitled");
    let t = w.untitled_tab();
    assert_eq!(t.name(), "Untitled 2");
    w.tabs.push(t);
    w.tabs.remove(0);
    assert_eq!(w.untitled_tab().name(), "Untitled", "a number given back is used again");
    w.tabs.clear();
    let fresh = w.untitled_tab();
    w.tabs.push(fresh);
    let dir = std::env::temp_dir().join(format!("vergence-edit-main-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("tabbed.txt");
    std::fs::write(&p, "a\n\tb\n\tc\n").unwrap();
    w.open_files(std::slice::from_ref(&p));
    assert_eq!(w.tabs.len(), 1, "the empty Untitled gave way");
    assert_eq!(w.tabs[0].name(), "tabbed.txt");
    assert!(!w.tabs[0].doc.borrow().options().spaces, "a file indented with tabs keeps tabs");
    assert!(!w.tabs[0].modified());
    // Opening it again brings the same tab forward.
    w.open_files(std::slice::from_ref(&p));
    assert_eq!(w.tabs.len(), 1);
    w.tabs[0].doc.borrow_mut().insert("x");
    assert!(w.tabs[0].modified());
    assert!(w.save(w.tabs[0].id));
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "xa\n\tb\n\tc\n");
    assert!(!w.tabs[0].modified());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_new_document_is_named_by_its_first_words() {
    let w = EditWindow::new(shared(), Vec::new());
    let t = &w.tabs[0];
    assert_eq!(suggested_name(t), "Untitled.txt");
    t.doc.borrow_mut().insert("\n  Shopping list: eggs/milk, and a great many other things to get\nmore");
    assert_eq!(suggested_name(t), "Shopping list eggsmilk, and a great many.txt");
}

#[test]
fn a_window_is_kept_and_put_back_with_its_unsaved_work() {
    let dir = std::env::temp_dir().join(format!("vergence-edit-keep-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("saved.txt");
    std::fs::write(&p, "one\ntwo\nthree\n").unwrap();
    let mut w = EditWindow::new(shared(), Vec::new());
    w.tabs[0].doc.borrow_mut().insert("a note not saved");
    w.open_files(std::slice::from_ref(&p));
    w.tabs[1]
        .doc
        .borrow_mut()
        .set_selection(vergence_widgets::edit::Pos::new(2, 1), vergence_widgets::edit::Pos::new(2, 1));
    w.token = "tok".into();
    let snap = w.snap();
    assert_eq!(snap.tabs.len(), 2);
    assert_eq!(snap.tabs[0].text.as_deref(), Some("a note not saved"));
    assert_eq!(snap.tabs[1].text, None, "a saved file is read again, not copied");
    let back = EditWindow::from_snap(shared(), &snap);
    assert_eq!(back.token, "tok");
    assert_eq!(back.tabs[0].name(), "Untitled");
    assert!(back.tabs[0].modified(), "unsaved work stays unsaved");
    assert_eq!(back.tabs[0].doc.borrow().text(), "a note not saved");
    assert_eq!(back.tabs[1].doc.borrow().caret(), vergence_widgets::edit::Pos::new(2, 1));
    assert_eq!(back.front, back.tabs[1].id, "the file was in front");
    // An empty Untitled is not kept.
    let empty = EditWindow::new(shared(), Vec::new());
    assert!(empty.snap().tabs.is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_file_changed_on_disk_is_read_again_or_asked_about() {
    let dir = std::env::temp_dir().join(format!("vergence-edit-disk-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("watched.txt");
    std::fs::write(&p, "first\n").unwrap();
    let mut w = EditWindow::new(shared(), Vec::new());
    w.open_files(std::slice::from_ref(&p));
    assert!(!w.check_disk(), "nothing changed");
    std::fs::write(&p, "second, longer\n").unwrap();
    assert!(w.check_disk());
    assert_eq!(w.tabs[0].doc.borrow().text(), "second, longer\n", "no changes here: read again");
    w.tabs[0].doc.borrow_mut().insert("mine ");
    std::fs::write(&p, "third, longer still\n").unwrap();
    assert!(w.check_disk());
    assert!(w.tabs[0].changed_on_disk, "changes here: asked");
    assert!(w.tabs[0].doc.borrow().text().starts_with("mine "));
    assert!(w.go_to("1:3"));
    assert_eq!(w.tabs[0].doc.borrow().caret(), vergence_widgets::edit::Pos::new(0, 2));
    assert!(!w.go_to("zero"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn closing_removes_tabs_and_moves_the_front() {
    let mut w = EditWindow::new(shared(), Vec::new());
    let (a, b) = (w.untitled_tab(), w.untitled_tab());
    let (ida, idb) = (a.id, b.id);
    w.tabs.push(a);
    w.tabs.push(b);
    w.front = ida;
    w.remove_tab(ida);
    assert_eq!(w.front, idb, "the next tab comes to the front");
    w.step_tab(1);
    assert_eq!(w.front, w.tabs[0].id, "and round");
}
