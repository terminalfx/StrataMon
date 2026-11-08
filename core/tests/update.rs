use minisign::{KeyPair, sign};
use std::io::Cursor;
use stratamon_core::update::*;

fn release(tag: &str) -> Vec<u8> {
    format!(r#"{{"tag_name":"{tag}","html_url":"https://evil.example/not-used"}}"#).into_bytes()
}

#[test]
fn reads_versions_shaped_like_a_release_tag() {
    assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
    assert_eq!(parse_version("0.2.0"), Some((0, 2, 0)));
    assert_eq!(parse_version("v10.20.30"), Some((10, 20, 30)));
}

#[test]
fn refuses_anything_else() {
    for text in ["", "v", "v1", "v1.2", "v1.2.3.4", "v1.2.x", "v1.2.-3", "1.2.3-beta", "v1..3", "latest", "v99999999999.0.0"] {
        assert_eq!(parse_version(text), None, "{text:?}");
    }
}

#[test]
fn compares_numbers_not_text() {
    assert!(parse_version("v0.10.0") > parse_version("v0.9.0"));
}

#[test]
fn a_newer_release_is_offered() {
    match interpret_release(200, &release("v0.3.0"), "0.2.0") {
        UpdateStatus::Available { version, tag, .. } => {
            assert_eq!(version, "0.3.0");
            assert_eq!(tag, "v0.3.0");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_link_is_built_from_known_parts_not_taken_from_the_reply() {
    let UpdateStatus::Available { link, .. } = interpret_release(200, &release("v0.3.0"), "0.2.0") else {
        panic!("expected an update");
    };
    assert!(link.starts_with("https://github.com/"));
    assert!(!link.contains("evil.example"));
}

#[test]
fn the_same_or_an_older_release_means_up_to_date() {
    assert_eq!(interpret_release(200, &release("v0.2.0"), "0.2.0"), UpdateStatus::UpToDate);
    assert_eq!(interpret_release(200, &release("v0.1.9"), "0.2.0"), UpdateStatus::UpToDate);
}

#[test]
fn each_problem_gets_a_plain_message() {
    let message = |status, body: &[u8]| match interpret_release(status, body, "0.2.0") {
        UpdateStatus::Failed(message) => message,
        other => panic!("{other:?}"),
    };

    assert!(message(404, b"").contains("No releases"));
    assert!(message(403, b"").contains("limiting"));
    assert!(message(429, b"").contains("limiting"));
    assert!(message(500, b"").contains("500"));
    assert!(message(200, b"<html>").contains("couldn't be read"));
    assert!(message(200, &release("nightly")).contains("can't read"));
    assert!(message(200, b"[]").contains("can't read"));
}

#[test]
fn asks_only_github_for_this_repository() {
    assert_eq!(latest_release_url("owner/StrataMon"), "https://api.github.com/repos/owner/StrataMon/releases/latest");
    let (file, signature) = download_urls("owner/StrataMon", "v0.3.0");
    assert_eq!(file, format!("https://github.com/owner/StrataMon/releases/download/v0.3.0/{ASSET}"));
    assert_eq!(signature, format!("{file}.minisig"));
}

fn comment(version: &str, file: &str) -> String {
    format!("version:{version} file:{file}")
}

#[test]
fn a_correct_signed_comment_passes() {
    assert_eq!(check_signed_comment(&comment("0.3.0", ASSET), "0.2.0", "v0.3.0"), Ok(()));
}

#[test]
fn the_other_platforms_file_is_refused() {
    let other = if ASSET.contains("win") { "StrataMon-linux-x64" } else { "StrataMon-win-x64.exe" };
    assert!(check_signed_comment(&comment("0.3.0", other), "0.2.0", "v0.3.0").unwrap_err().contains("different platform"));
}

#[test]
fn an_old_signed_release_cannot_be_replayed() {
    assert!(check_signed_comment(&comment("0.2.0", ASSET), "0.2.0", "v0.2.0").unwrap_err().contains("isn't newer"));
    assert!(check_signed_comment(&comment("0.1.0", ASSET), "0.2.0", "v0.3.0").unwrap_err().contains("isn't newer"));
}

#[test]
fn a_signature_from_a_different_release_is_refused() {
    assert!(check_signed_comment(&comment("0.3.0", ASSET), "0.2.0", "v0.4.0").unwrap_err().contains("doesn't match"));
}

#[test]
fn a_comment_missing_its_details_is_refused() {
    assert!(check_signed_comment("", "0.2.0", "v0.3.0").is_err());
    assert!(check_signed_comment(&format!("file:{ASSET}"), "0.2.0", "v0.3.0").is_err());
    assert!(check_signed_comment("version:0.3.0", "0.2.0", "v0.3.0").is_err());
}

// The whole signature check, with real keys made for the test.

const FILE: &[u8] = b"a new StrataMon";

fn keys() -> (KeyPair, String) {
    let pair = KeyPair::generate_unencrypted_keypair().unwrap();
    let public = pair.pk.to_base64();
    (pair, public)
}

fn signed(pair: &KeyPair, data: &[u8], comment: &str) -> String {
    sign(None, &pair.sk, Cursor::new(data), Some(comment), None).unwrap().into_string()
}

fn good_comment() -> String {
    comment("0.3.0", ASSET)
}

#[test]
fn a_correctly_signed_download_passes() {
    let (pair, public) = keys();
    let signature = signed(&pair, FILE, &good_comment());
    assert_eq!(verify_download(FILE, &signature, &public, "0.2.0", "v0.3.0"), Ok(()));
}

#[test]
fn a_tampered_file_is_refused() {
    let (pair, public) = keys();
    let signature = signed(&pair, FILE, &good_comment());
    let error = verify_download(b"a new StrataMoN", &signature, &public, "0.2.0", "v0.3.0").unwrap_err();
    assert!(error.contains("doesn't match"));
}

#[test]
fn a_signature_from_another_key_is_refused() {
    let (attacker, _) = keys();
    let (_, public) = keys();
    let signature = signed(&attacker, FILE, &good_comment());
    assert!(verify_download(FILE, &signature, &public, "0.2.0", "v0.3.0").is_err());
}

// This is what stops an old signed release being relabeled as a new one.
#[test]
fn editing_the_signed_comment_is_refused() {
    let (pair, public) = keys();
    let signature = signed(&pair, FILE, &comment("0.1.0", ASSET));
    let relabeled = signature.replace("version:0.1.0", "version:0.3.0");

    assert_ne!(signature, relabeled);
    assert!(verify_download(FILE, &relabeled, &public, "0.2.0", "v0.3.0").is_err());
}

#[test]
fn a_correctly_signed_old_release_is_refused() {
    let (pair, public) = keys();
    let signature = signed(&pair, FILE, &comment("0.1.0", ASSET));
    assert!(verify_download(FILE, &signature, &public, "0.2.0", "v0.1.0").is_err());
}

#[test]
fn a_correctly_signed_file_for_the_other_platform_is_refused() {
    let (pair, public) = keys();
    let other = if ASSET.contains("win") { "StrataMon-linux-x64" } else { "StrataMon-win-x64.exe" };
    let signature = signed(&pair, FILE, &comment("0.3.0", other));
    assert!(verify_download(FILE, &signature, &public, "0.2.0", "v0.3.0").is_err());
}

#[test]
fn garbage_signatures_and_keys_are_refused_without_crashing() {
    let (pair, public) = keys();
    let signature = signed(&pair, FILE, &good_comment());

    assert!(verify_download(FILE, "", &public, "0.2.0", "v0.3.0").is_err());
    assert!(verify_download(FILE, "not a signature", &public, "0.2.0", "v0.3.0").is_err());
    assert!(verify_download(FILE, &signature, "not a key", "0.2.0", "v0.3.0").is_err());
    assert!(verify_download(FILE, &signature, "", "0.2.0", "v0.3.0").is_err());
}

#[test]
fn an_empty_download_is_refused() {
    let (pair, public) = keys();
    let signature = signed(&pair, FILE, &good_comment());
    assert!(verify_download(b"", &signature, &public, "0.2.0", "v0.3.0").is_err());
}
