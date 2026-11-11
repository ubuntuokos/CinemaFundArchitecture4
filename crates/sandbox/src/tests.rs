use std::path::PathBuf;

use super::*;

fn ctx() -> Context {
    Context {
        project_dirs: vec![PathBuf::from("/work/film")],
        media: vec![PathBuf::from("/media/rushes")],
        media_file: Some(PathBuf::from("/media/rushes/a.mov")),
        cache_dir: Some(PathBuf::from("/home/u/.cache/cfa4")),
        config_dir: Some(PathBuf::from("/home/u/.config/cfa4")),
        export_targets: vec![PathBuf::from("/work/out/film.mp4")],
        app_dirs: vec![PathBuf::from("/opt/cfa4")],
        helper_exes: vec![PathBuf::from("/opt/cfa4/bin/cfa4-media-host")],
        loopback_ports: vec![9876],
        download_target: Some(PathBuf::from("/home/u/.cache/cfa4/models")),
        model_source: Some(PathBuf::from("/tmp/m.ckpt")),
        model_output: Some(PathBuf::from("/tmp/m.safetensors")),
        ..Context::default()
    }
}

fn full_support() -> Support {
    Support { landlock_abi: 8, seccomp: true, apparmor: true, userns_restricted: true, ..Support::default() }
}

#[test]
fn profiles_follow_plan_table() {
    let c = ctx();
    let p = |k| Profile::for_process(k, &c);

    // media host: only the opened file, writes nothing, no network, starts nothing (D2)
    let m = p(ProcessKind::MediaHost);
    assert_eq!(m.read, vec![PathBuf::from("/opt/cfa4"), PathBuf::from("/media/rushes/a.mov")]);
    assert!(m.write.is_empty() && m.exec.is_empty());
    assert_eq!(m.network, Network::None);
    assert!(!m.covers(&PathBuf::from("/media/rushes/b.mov")), "only the one file");

    // engine: loopback only, its own helpers, writes project/cache/export (D5)
    let e = p(ProcessKind::Engine);
    assert_eq!(e.network, Network::Loopback { ports: vec![9876] });
    assert_eq!(e.exec, c.helper_exes);
    assert!(e.write.contains(&PathBuf::from("/work/film")) && e.write.contains(&PathBuf::from("/work/out/film.mp4")));
    assert!(e.read.contains(&PathBuf::from("/media/rushes")));

    // downloader: reads nothing of the user's, writes only the target, HTTPS out (D5)
    let f = p(ProcessKind::Fetch);
    assert_eq!(f.read, vec![PathBuf::from("/opt/cfa4")]);
    assert_eq!(f.write, vec![PathBuf::from("/home/u/.cache/cfa4/models")]);
    assert_eq!(f.network, Network::Outbound { ports: vec![443] });
    assert!(f.exec.is_empty());

    // plug-ins and the model converter: no network, no program start (D7)
    for k in [ProcessKind::PluginHost, ProcessKind::ModelConverter] {
        assert_eq!(p(k).network, Network::None, "{k:?}");
        assert!(p(k).exec.is_empty(), "{k:?}");
    }
    assert_eq!(p(ProcessKind::ModelConverter).write, vec![PathBuf::from("/tmp/m.safetensors")]);
    assert!(p(ProcessKind::PluginHost).write.is_empty());

    // only the studio host goes out to the network
    for k in ProcessKind::ALL {
        let any = p(k).network == Network::Any;
        assert_eq!(any, matches!(k, ProcessKind::StudioHost | ProcessKind::BrowserSource), "{k:?}");
    }
}

#[test]
fn no_profile_reads_home_by_default() {
    let empty = Context::default();
    for k in ProcessKind::ALL {
        let p = Profile::for_process(k, &empty);
        assert!(p.read.is_empty() && p.write.is_empty() && p.exec.is_empty(), "{k:?}: {p:?}");
    }
    for s in profile::SYSTEM_READ {
        assert!(!s.starts_with("/home") && !s.starts_with("/root") && *s != "/proc", "{s}");
    }
}

#[test]
fn kind_ids_round_trip() {
    for k in ProcessKind::ALL {
        assert_eq!(ProcessKind::from_id(k.id()), Some(k));
        assert_eq!(serde_json::to_value(k).unwrap(), serde_json::json!(k.id()));
    }
    assert_eq!(ProcessKind::from_id("nope"), None);
}

#[test]
fn strict_mode_names_what_is_missing() {
    let c = ctx();
    let engine = Profile::for_process(ProcessKind::Engine, &c);
    assert!(missing(&engine, &full_support()).is_empty());

    let none = Support::default();
    let m = missing(&engine, &none);
    assert_eq!(m.len(), 3, "{m:?}");
    assert!(m.iter().any(|s| s.contains("seccomp")));

    // ABI 3: file rules but no port rules; port-limited profiles miss them, unrestricted ones don't
    let old = Support { landlock_abi: 3, seccomp: true, ..Support::default() };
    assert_eq!(missing(&engine, &old), vec!["Landlock ABI 4 (network port rules)".to_string()]);
    assert!(missing(&Profile::for_process(ProcessKind::StudioHost, &c), &old).is_empty());

    // the browser source needs AppArmor for Chromium's sandbox (AppImage has none: D4, VIII)
    let browser = Profile::for_process(ProcessKind::BrowserSource, &c);
    assert!(missing(&browser, &full_support()).is_empty());
    assert_eq!(missing(&browser, &Support { apparmor: false, ..full_support() }).len(), 1);
}

#[test]
fn browser_source_is_not_applied_here() {
    let p = Profile::for_process(ProcessKind::BrowserSource, &Context::default());
    assert!(matches!(apply(&p, Mode::Strict), Err(SandboxError::External("browserSource"))));
}

#[test]
fn status_lists_every_process_kind() {
    let v = status(&ctx());
    let procs = v["processes"].as_array().unwrap();
    assert_eq!(procs.len(), ProcessKind::ALL.len());
    for p in procs {
        for k in ["kind", "label", "read", "write", "exec", "network", "gpu", "canStart", "missing"] {
            assert!(!p[k].is_null(), "{k} in {p}");
        }
    }
    assert!(v["support"]["landlockAbi"].is_u64());
    assert!(v["support"]["seccomp"].is_boolean());
    assert!(v["self"]["seccompMode"].is_u64());
    assert_eq!(v["unsupported"], false);
}

#[cfg(target_os = "linux")]
#[test]
fn seccomp_programs_build_for_every_kind() {
    let c = ctx();
    for k in ProcessKind::ALL {
        let prog = linux::seccomp_program(&Profile::for_process(k, &c)).unwrap();
        assert!(!prog.is_empty(), "{k:?}");
    }
    // the no-network filter is longer (socket family checks)
    let none = linux::seccomp_program(&Profile::for_process(ProcessKind::MediaHost, &c)).unwrap();
    let any = linux::seccomp_program(&Profile::for_process(ProcessKind::StudioHost, &c)).unwrap();
    assert!(none.len() > any.len());
}
