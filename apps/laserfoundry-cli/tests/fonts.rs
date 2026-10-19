//! `fonts download` against a local stand-in for GitHub.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output};

mod download_tests {
    use super::*;

    #[test]
    fn the_foundry_fonts_are_downloaded_to_the_chosen_folder_and_previews_are_left_out() {
        let dest = temp_dir("download");
        let (tree, raw) =
            serve(&[("fonts/foundry-block/font.json", b"{}"), ("fonts/foundry-block/preview.png", b"png"), ("inkstitch/a/ltr.svg", b"<svg/>")]);
        let out = cli(&["fonts", "download", "--to", dest.to_str().unwrap(), "--tree-url", &tree, "--raw-base", &raw]);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("1 downloaded"), "{text}");
        assert!(text.contains("Ink/Stitch fonts are not drawable yet"), "the limitation is stated: {text}");
        assert!(dest.join("fonts/foundry-block/font.json").exists());
        assert!(!dest.join("fonts/foundry-block/preview.png").exists());
        assert!(!dest.join("inkstitch").exists(), "--all was not given");

        let out = cli(&["fonts", "download", "--all", "--to", dest.to_str().unwrap(), "--tree-url", &tree, "--raw-base", &raw]);
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("1 downloaded, 1 already there"), "{}", String::from_utf8_lossy(&out.stdout));
        assert!(dest.join("inkstitch/a/ltr.svg").exists());
    }

    #[test]
    fn bad_options_and_an_unreachable_server_are_errors() {
        let dest = temp_dir("errors");
        for args in [&["fonts"][..], &["fonts", "bogus"], &["fonts", "download", "--nope"], &["fonts", "download", "--to"]] {
            assert!(!cli(args).status.success(), "{args:?}");
        }
        let out =
            cli(&["fonts", "download", "--to", dest.to_str().unwrap(), "--tree-url", "http://127.0.0.1:9/tree", "--raw-base", "http://127.0.0.1:9"]);
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("could not reach"), "{}", String::from_utf8_lossy(&out.stderr));
        let out = cli(&["fonts", "path"]);
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("LaserFoundry"));
    }
}

fn cli(args: &[&str]) -> Output {
    let home = temp_dir("home");
    Command::new(env!("CARGO_BIN_EXE_laserfoundry-cli"))
        .args(args)
        .env("HOME", &home)
        .env("XDG_DATA_HOME", &home)
        .env("APPDATA", &home)
        .env_remove("LASERFOUNDRY_TRANSLATION")
        .output()
        .unwrap()
}

/// Serves a tree listing and the files; returns (tree url, raw base).
fn serve(files: &[(&str, &[u8])]) -> (String, String) {
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    let mut entries = Vec::new();
    for (path, body) in files {
        routes.insert(format!("/raw/{path}"), body.to_vec());
        entries.push(serde_json::json!({ "path": path, "type": "blob", "size": body.len() }));
    }
    routes.insert("/tree".into(), serde_json::json!({ "tree": entries, "truncated": false }).to_string().into_bytes());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let path = line.split_whitespace().nth(1).unwrap_or("").split('?').next().unwrap_or("").to_string();
            while reader.read_line(&mut String::new()).unwrap_or(0) > 2 {}
            let mut stream = reader.into_inner();
            let _ = match routes.get(&path) {
                Some(body) => {
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    stream.write_all(body)
                }
                None => write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
            };
        }
    });
    (format!("{address}/tree?recursive=1"), format!("{address}/raw"))
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("laserfoundry-cli-fonts-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
