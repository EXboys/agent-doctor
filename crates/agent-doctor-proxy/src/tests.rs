use super::*;
use crate::system::{no_proxy_entry, parse_scutil, parse_windows};

fn vars(pairs: &[(&str, &str)]) -> Vars {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn up(_: &str) -> bool {
    true
}

fn down(_: &str) -> bool {
    false
}

fn clash_system() -> SystemProxy {
    let hp = HostPort {
        host: "127.0.0.1".into(),
        port: 7890,
    };
    SystemProxy {
        http: Some(hp.clone()),
        https: Some(hp),
        socks: None,
        pac: false,
        exceptions: vec![".local".into(), "169.254.0.0/16".into()],
    }
}

#[test]
fn nothing_found_sets_nothing() {
    let plan = resolve(&Inputs::default(), &up);
    assert_eq!(plan.source, ProxySource::None);
    assert!(plan.set.is_empty() && plan.remove.is_empty());
}

#[test]
fn proxy_already_in_process_is_left_alone() {
    let inputs = Inputs {
        process: vars(&[("HTTPS_PROXY", "http://10.0.0.2:3128")]),
        shell: Some(vars(&[("https_proxy", "http://127.0.0.1:7890")])),
        system: Some(clash_system()),
        ..Inputs::default()
    };
    let plan = resolve(&inputs, &up);
    assert_eq!(plan.source, ProxySource::Inherited);
    assert!(
        plan.set.is_empty(),
        "inherited variables reach the child unchanged"
    );
}

#[test]
fn shell_proxy_fills_both_cases_and_keeps_loopback_direct() {
    let inputs = Inputs {
        shell: Some(vars(&[
            ("https_proxy", "127.0.0.1:7890"),
            ("no_proxy", "corp.example"),
            ("ANTHROPIC_API_KEY", "sk-should-not-travel"),
        ])),
        ..Inputs::default()
    };
    let plan = resolve(&inputs, &up);
    assert_eq!(plan.source, ProxySource::LoginShell);
    for key in ["https_proxy", "HTTPS_PROXY", "http_proxy", "HTTP_PROXY"] {
        assert_eq!(plan.set[key], "http://127.0.0.1:7890", "{key}");
    }
    assert_eq!(plan.set["NO_PROXY"], "corp.example,localhost,127.0.0.1,::1");
    assert_eq!(plan.set["no_proxy"], plan.set["NO_PROXY"]);
    assert!(!plan.set.contains_key("ANTHROPIC_API_KEY"));
    assert!(
        !plan.set.contains_key("ALL_PROXY"),
        "no all_proxy unless the shell had one"
    );
}

#[test]
fn dead_shell_proxy_falls_to_system_proxy() {
    let inputs = Inputs {
        shell: Some(vars(&[("https_proxy", "http://127.0.0.1:7890")])),
        system: Some(SystemProxy {
            http: Some(HostPort {
                host: "127.0.0.1".into(),
                port: 1087,
            }),
            ..SystemProxy::default()
        }),
        ..Inputs::default()
    };
    let alive = |url: &str| url.ends_with(":1087");
    let plan = resolve(&inputs, &alive);
    assert_eq!(plan.source, ProxySource::System);
    assert_eq!(plan.set["HTTPS_PROXY"], "http://127.0.0.1:1087");
    assert_eq!(plan.notes, vec![Note::ShellProxyUnreachable]);
}

#[test]
fn every_dead_proxy_means_direct() {
    let inputs = Inputs {
        shell: Some(vars(&[("https_proxy", "http://127.0.0.1:7890")])),
        system: Some(clash_system()),
        ..Inputs::default()
    };
    let plan = resolve(&inputs, &down);
    assert_eq!(plan.source, ProxySource::None);
    assert!(plan.set.is_empty());
    assert_eq!(
        plan.notes,
        vec![Note::ShellProxyUnreachable, Note::SystemProxyUnreachable]
    );
}

#[test]
fn system_proxy_carries_its_exceptions() {
    let inputs = Inputs {
        system: Some(clash_system()),
        ..Inputs::default()
    };
    let plan = resolve(&inputs, &up);
    assert_eq!(plan.source, ProxySource::System);
    assert_eq!(plan.set["http_proxy"], "http://127.0.0.1:7890");
    assert_eq!(
        plan.set["NO_PROXY"],
        ".local,169.254.0.0/16,localhost,127.0.0.1,::1"
    );
}

#[test]
fn socks_only_and_pac_stay_direct_with_a_note() {
    let inputs = Inputs {
        system: Some(SystemProxy {
            socks: Some(HostPort {
                host: "127.0.0.1".into(),
                port: 1080,
            }),
            pac: true,
            ..SystemProxy::default()
        }),
        ..Inputs::default()
    };
    let plan = resolve(&inputs, &up);
    assert_eq!(plan.source, ProxySource::None);
    assert!(plan.set.is_empty());
    assert_eq!(
        plan.notes,
        vec![Note::SystemSocksOnly, Note::SystemPacUnsupported]
    );
}

#[test]
fn explicit_url_wins_and_is_not_probed() {
    let inputs = Inputs {
        explicit: Some(Explicit::Url("http://10.0.0.9:8080".into())),
        process: vars(&[("HTTPS_PROXY", "http://other:1")]),
        ..Inputs::default()
    };
    let plan = resolve(&inputs, &down);
    assert_eq!(plan.source, ProxySource::Explicit);
    assert_eq!(plan.set["HTTPS_PROXY"], "http://10.0.0.9:8080");
}

#[test]
fn explicit_direct_strips_every_proxy() {
    let inputs = Inputs {
        explicit: Some(Explicit::Direct),
        shell: Some(vars(&[("SSL_CERT_FILE", "/tmp/ca.pem")])),
        ..Inputs::default()
    };
    let plan = resolve(&inputs, &up);
    assert_eq!(plan.source, ProxySource::Direct);
    assert!(plan.remove.iter().any(|key| key == "HTTPS_PROXY"));
    assert!(plan.remove.iter().any(|key| key == "no_proxy"));
    assert!(plan.set.is_empty());
}

#[test]
fn shell_certificates_travel_unless_the_process_has_them() {
    let inputs = Inputs {
        process: vars(&[("SSL_CERT_FILE", "/etc/own.pem")]),
        shell: Some(vars(&[
            ("SSL_CERT_FILE", "/tmp/shell.pem"),
            ("NODE_EXTRA_CA_CERTS", "/tmp/corp.pem"),
        ])),
        ..Inputs::default()
    };
    let plan = resolve(&inputs, &up);
    assert_eq!(plan.set["NODE_EXTRA_CA_CERTS"], "/tmp/corp.pem");
    assert!(!plan.set.contains_key("SSL_CERT_FILE"));
}

#[test]
fn send_path_reads_the_stored_plan() {
    let mut cached = ProxyPlan::empty(ProxySource::System);
    cached
        .set
        .insert("HTTPS_PROXY".into(), "http://127.0.0.1:7890".into());
    let plan = plan_for_child(None, &Vars::new(), &cached);
    assert_eq!(plan.source, ProxySource::System);
    assert_eq!(plan.set["HTTPS_PROXY"], "http://127.0.0.1:7890");
}

#[test]
fn a_proxy_already_on_the_process_beats_the_stored_plan() {
    let mut cached = ProxyPlan::empty(ProxySource::System);
    cached
        .set
        .insert("HTTPS_PROXY".into(), "http://127.0.0.1:7890".into());
    let plan = plan_for_child(
        None,
        &vars(&[("https_proxy", "http://10.0.0.2:3128")]),
        &cached,
    );
    assert_eq!(plan.source, ProxySource::Inherited);
    assert!(plan.set.is_empty());
}

#[test]
fn summary_hides_credentials() {
    let inputs = Inputs {
        explicit: Some(Explicit::Url("http://bob:secret@10.0.0.9:8080".into())),
        ..Inputs::default()
    };
    let summary = resolve(&inputs, &up).summary();
    assert_eq!(summary, "source=explicit proxy=http://***@10.0.0.9:8080");
}

#[test]
fn apply_sets_and_removes() {
    let mut plan = ProxyPlan::empty(ProxySource::System);
    plan.set
        .insert("HTTPS_PROXY".into(), "http://127.0.0.1:7890".into());
    plan.remove.push("ALL_PROXY".into());
    // Built only to check env_remove / env; never spawned.
    let mut cmd = Command::new("true"); // raw-spawn-ok
    plan.apply(&mut cmd);
    let envs: Vec<_> = cmd.get_envs().collect();
    assert!(envs.contains(&(
        std::ffi::OsStr::new("HTTPS_PROXY"),
        Some(std::ffi::OsStr::new("http://127.0.0.1:7890"))
    )));
    assert!(envs.contains(&(std::ffi::OsStr::new("ALL_PROXY"), None)));
}

#[test]
fn scutil_clash_output() {
    let text = "<dictionary> {
  ExceptionsList : <array> {
    0 : *.local
    1 : 169.254/16
  }
  FTPPassive : 1
  HTTPEnable : 1
  HTTPPort : 7890
  HTTPProxy : 127.0.0.1
  HTTPSEnable : 1
  HTTPSPort : 7890
  HTTPSProxy : 127.0.0.1
  SOCKSEnable : 1
  SOCKSPort : 7891
  SOCKSProxy : 127.0.0.1
}";
    let proxy = parse_scutil(text).expect("proxy");
    assert_eq!(proxy.https.as_ref().unwrap().url(), "http://127.0.0.1:7890");
    assert_eq!(proxy.socks.as_ref().unwrap().port, 7891);
    assert_eq!(proxy.exceptions, vec![".local", "169.254.0.0/16"]);
    assert!(!proxy.pac);
}

#[cfg(target_os = "macos")]
#[test]
fn live_scutil_read_matches_its_output() {
    // Compared with system::read; output is captured.
    let direct = std::process::Command::new("/usr/sbin/scutil") // raw-spawn-ok
        .arg("--proxy")
        .output()
        .expect("scutil");
    assert!(direct.status.success());
    let expected = parse_scutil(&String::from_utf8_lossy(&direct.stdout));
    assert_eq!(crate::system::read(), expected);
}

#[test]
fn scutil_without_proxy_is_none() {
    let text = "<dictionary> {
  ExceptionsList : <array> {
    0 : *.local
    1 : 169.254/16
  }
  FTPPassive : 1
}";
    assert_eq!(parse_scutil(text), None);
}

#[test]
fn scutil_disabled_entries_are_ignored() {
    let text = "<dictionary> {
  HTTPEnable : 0
  HTTPPort : 7890
  HTTPProxy : 127.0.0.1
  ProxyAutoConfigEnable : 1
  ProxyAutoConfigURLString : http://127.0.0.1:7890/pac
}";
    let proxy = parse_scutil(text).expect("pac only");
    assert!(proxy.http.is_none() && proxy.pac);
}

#[test]
fn windows_single_and_per_scheme_servers() {
    let single = parse_windows("127.0.0.1:7890", "localhost;127.*;<local>", false).unwrap();
    assert_eq!(
        single.https.as_ref().unwrap().url(),
        "http://127.0.0.1:7890"
    );
    assert_eq!(single.exceptions, vec!["localhost", "127.*"]);

    let split = parse_windows(
        "http=10.0.0.1:80;https=10.0.0.1:443;socks=10.0.0.1:1080",
        "",
        false,
    )
    .unwrap();
    assert_eq!(split.http.unwrap().port, 80);
    assert_eq!(split.https.unwrap().port, 443);
    assert_eq!(split.socks.unwrap().port, 1080);

    assert_eq!(parse_windows("", "", false), None);
    assert!(parse_windows("", "", true).unwrap().pac);
}

#[test]
fn no_proxy_entry_forms() {
    assert_eq!(no_proxy_entry("*.local").as_deref(), Some(".local"));
    assert_eq!(no_proxy_entry("10/8").as_deref(), Some("10.0.0.0/8"));
    assert_eq!(
        no_proxy_entry("192.168.0.0/16").as_deref(),
        Some("192.168.0.0/16")
    );
    assert_eq!(no_proxy_entry("<local>"), None);
}

#[test]
fn probe_addresses() {
    use crate::probe::socket_addr;
    assert_eq!(
        socket_addr("http://127.0.0.1:7890").unwrap().to_string(),
        "127.0.0.1:7890"
    );
    assert_eq!(
        socket_addr("http://u:p@localhost:8080/")
            .unwrap()
            .to_string(),
        "127.0.0.1:8080"
    );
    assert_eq!(socket_addr("http://[::1]:7890").unwrap().port(), 7890);
    assert!(
        socket_addr("http://proxy.corp:3128").is_none(),
        "host names are not dialed"
    );
}

#[test]
fn probe_sees_a_closed_and_an_open_port() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let open = format!("http://{}", listener.local_addr().unwrap());
    assert!(crate::probe::alive(&open));
    drop(listener);
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    assert!(!crate::probe::alive(&format!("http://127.0.0.1:{port}")));
}

#[cfg(unix)]
mod shell_capture {
    use std::path::Path;
    use std::time::Duration;

    use crate::shell::unix::{capture_with, parse};

    #[test]
    fn parse_cuts_between_markers() {
        let out = "prompt junk\nMK\nhttps_proxy=http://127.0.0.1:7890\nPATH=/bin\nMK\ntrailing";
        let vars = parse(out, "MK").unwrap();
        assert_eq!(vars["https_proxy"], "http://127.0.0.1:7890");
        assert!(
            parse("MK\nA=1\n", "MK").is_none(),
            "an unclosed block is not trusted"
        );
    }

    #[test]
    fn zsh_rc_proxy_is_read() {
        let zsh = Path::new("/bin/zsh");
        if !zsh.exists() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".zshrc"),
            "echo noisy prompt\nexport https_proxy=http://127.0.0.1:7890\n",
        )
        .unwrap();
        let zdotdir = dir.path().to_str().unwrap();
        let vars =
            capture_with(zsh, &[("ZDOTDIR", zdotdir)], Duration::from_secs(10)).expect("shell env");
        assert_eq!(vars["https_proxy"], "http://127.0.0.1:7890");
    }

    #[test]
    fn a_hanging_rc_file_is_cut_off() {
        let zsh = Path::new("/bin/zsh");
        if !zsh.exists() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".zshrc"), "sleep 30\n").unwrap();
        std::fs::write(dir.path().join(".zprofile"), "sleep 30\n").unwrap();
        let zdotdir = dir.path().to_str().unwrap();
        let started = std::time::Instant::now();
        let vars = capture_with(zsh, &[("ZDOTDIR", zdotdir)], Duration::from_secs(1));
        assert!(vars.is_none());
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "{:?}",
            started.elapsed()
        );
    }
}
