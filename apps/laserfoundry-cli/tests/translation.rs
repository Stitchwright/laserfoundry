//! The command line translates its own texts with the same catalogs as the desktop app.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

mod catalog_tests {
    use super::*;

    #[test]
    fn info_and_errors_follow_the_chosen_catalog() {
        let dir = temp_dir("catalog");
        let svg = dir.join("a.svg");
        std::fs::write(&svg, r#"<svg xmlns="http://www.w3.org/2000/svg" width="20mm" height="10mm"><rect width="20" height="10"/></svg>"#).unwrap();
        let catalog = write_catalog(
            &dir,
            r#"{"name":"Test","strings":{"shapes: {count}":"Formen: {count}","error: {message}":"Fehler: {message}","unknown option {option}":"unbekannte Option {option}"}}"#,
        );
        let out = cli(&dir, &["--translation", path(&catalog), "info", path(&svg)]);
        assert!(out.status.success(), "{}", stderr(&out));
        let text = stdout(&out);
        assert!(text.contains("Formen: 1"), "{text}");
        assert!(text.contains("size: ") && text.contains(" mm"), "entries the catalog lacks stay English: {text}");

        let out = cli(&dir, &["--translation", path(&catalog), "gcode", path(&svg), "--nope", "1"]);
        assert!(!out.status.success());
        assert!(stderr(&out).contains("Fehler: unbekannte Option --nope"), "{}", stderr(&out));
    }

    #[test]
    fn without_a_catalog_everything_is_english_and_the_environment_can_name_one() {
        let dir = temp_dir("english");
        let out = cli(&dir, &["bogus"]);
        assert!(stderr(&out).starts_with("error: usage: laserfoundry-cli"), "{}", stderr(&out));
        let catalog = write_catalog(&dir, r#"{"strings":{"error: {message}":"Fehler: {message}"}}"#);
        let out = Command::new(env!("CARGO_BIN_EXE_laserfoundry-cli"))
            .arg("bogus")
            .env("HOME", &dir)
            .env("XDG_DATA_HOME", &dir)
            .env("APPDATA", &dir)
            .env("LASERFOUNDRY_TRANSLATION", &catalog)
            .output()
            .unwrap();
        assert!(stderr(&out).starts_with("Fehler:"), "{}", stderr(&out));
    }

    #[test]
    fn the_desktop_apps_catalog_is_used_when_nothing_else_is_named() {
        let dir = temp_dir("default");
        let home = dir.join("LaserFoundry");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("translation.json"), r#"{"strings":{"error: {message}":"Fehler: {message}"}}"#).unwrap();
        let out = cli(&dir, &["bogus"]);
        assert!(stderr(&out).starts_with("Fehler:"), "{}", stderr(&out));
    }
}

mod template_tests {
    use super::*;

    #[test]
    fn the_template_lists_every_text_the_tool_shows() {
        let dir = temp_dir("template");
        let out = cli(&dir, &["translation-template"]);
        assert!(out.status.success(), "{}", stderr(&out));
        let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
        let strings = v["strings"].as_object().unwrap();
        for key in ["shapes: {count}", "error: {message}", "bad bed width", "  info <input>         shape count, size, estimated time"] {
            assert!(strings.contains_key(key), "missing {key:?}");
        }
        assert!(strings.contains_key("shape {id} does not exist"), "engine messages are included");
    }
}

mod hostile_tests {
    use super::*;

    #[test]
    fn a_missing_or_broken_chosen_catalog_is_an_error_but_a_broken_default_is_ignored() {
        let dir = temp_dir("hostile");
        let out = cli(&dir, &["--translation", path(&dir.join("missing.json")), "commands"]);
        assert!(!out.status.success());
        assert!(stderr(&out).contains("translation"), "{}", stderr(&out));
        let bad = write_catalog(&dir, "{ not json");
        assert!(!cli(&dir, &["--translation", path(&bad), "commands"]).status.success());
        assert!(!cli(&dir, &["--translation"]).status.success(), "the flag needs a file");
        let home = dir.join("LaserFoundry");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("translation.json"), "{ not json").unwrap();
        assert!(cli(&dir, &["commands"]).status.success(), "a broken default catalog must not stop the tool");
    }
}

fn cli(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_laserfoundry-cli"))
        .args(args)
        .env("HOME", home)
        .env("XDG_DATA_HOME", home)
        .env("APPDATA", home)
        .env_remove("LASERFOUNDRY_TRANSLATION")
        .output()
        .unwrap()
}

fn write_catalog(dir: &Path, text: &str) -> PathBuf {
    let file = dir.join("catalog.json");
    std::fs::write(&file, text).unwrap();
    file
}

fn path(p: &Path) -> &str {
    p.to_str().unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("laserfoundry-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
