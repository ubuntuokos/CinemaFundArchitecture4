//! CFX escape and resource tests (plan CFA4-AX-INTERPRETER-001, X5: N3-S1′, N3-S2′).
//!
//! The language has no I/O, no clock, no recursion and no unbounded loop, so these tests do not probe a sandbox: they
//! check that every attempt to *write* such a thing is a compile error, that every resource limit stops a hostile
//! program inside its bound, and that running many hostile programs does not grow the process. Only the public API is
//! used, as the engine uses it.

use std::time::{Duration, Instant};

use cfa4_expr::{Context, DEFAULT_FUEL, Env, MAX_FUEL, Schema, Ty, Value, compile};

fn schema() -> Schema {
    Schema { context: Context::Volume, attrs: vec![("density".into(), Ty::Float), ("out".into(), Ty::Float), ("vel".into(), Ty::Vec3)] }
}

fn zeros(s: &Schema) -> Vec<Value> {
    s.attrs.iter().map(|(_, t)| Value::zero(*t)).collect()
}

/// Compiles and, if that works, runs once with `fuel`; returns the error code or `"ok"`.
fn outcome(src: &str, fuel: u32) -> String {
    let s = schema();
    match compile(src, &s) {
        Err(d) => d.code.to_string(),
        Ok(p) => {
            let mut vm = p.new_vm();
            let mut out = zeros(&s);
            match p.run(&mut vm, fuel, &Env::default(), &[], &zeros(&s), &mut out) {
                Ok(_) => "ok".into(),
                Err(d) => d.code.to_string(),
            }
        }
    }
}

/// N3-S1′: file, network, process, time, environment and escape syntaxes of every language family a script
/// could be written in. Not one may compile.
#[test]
fn no_io_syntax_compiles() {
    let attempts = [
        // calls that would read or write files, talk to the network, start processes, read the clock or the environment
        r#"writes @out; @out = open("/etc/passwd");"#,
        r#"writes @out; @out = fopen("/etc/passwd", "r");"#,
        r#"writes @out; @out = read_file("/etc/shadow");"#,
        r#"writes @out; @out = system("id");"#,
        r#"writes @out; @out = exec("/bin/sh");"#,
        r#"writes @out; @out = popen("ls", "r");"#,
        r#"writes @out; @out = socket(2, 1, 0);"#,
        r#"writes @out; @out = connect(1, 2, 3);"#,
        r#"writes @out; @out = http_get("http://example.com");"#,
        r#"writes @out; @out = time();"#,
        r#"writes @out; @out = clock();"#,
        r#"writes @out; @out = now();"#,
        r#"writes @out; @out = getenv("HOME");"#,
        r#"writes @out; @out = environ();"#,
        r#"writes @out; @out = rand_os();"#,
        r#"writes @out; @out = malloc(1000000000);"#,
        r#"writes @out; @out = sleep(100);"#,
        r#"writes @out; @out = print(1.0);"#,
        r#"writes @out; @out = eval("1.0");"#,
        r#"writes @out; @out = __import__("os");"#,
        // statements and declarations of other languages
        "import os;",
        "#include <stdio.h>",
        "use std::fs;",
        "require('fs');",
        "extern fn read();",
        "fn f() { f(); }",
        "float f(float x) { return f(x); }",
        "while (true) { }",
        "do { } while (true);",
        "loop { }",
        "goto top;",
        "for (;;) { }",
        "for i in 0..1000000000 { }",
        "label: writes @out;",
        "asm(\"syscall\");",
        "unsafe { }",
        // strings, backticks, shell and path syntax, escapes and encodings
        "writes @out; @out = \"string\";",
        "writes @out; @out = 'c';",
        "writes @out; @out = `ls`;",
        "writes @out; @out = $(id);",
        "writes @out; @out = ${HOME};",
        "writes @out; @out = @@out;",
        "writes @out; @out = @/etc/passwd;",
        "writes @out; @out = ../../etc/passwd;",
        "writes @out; @out = 0x7fffffff;",
        "writes @out; @out = 1e999999;",
        "writes @out; @out = 1.0.0;",
        "writes @out; \u{0}",
        "writes @out; \u{202e}@out = 1.0;",
        "writes @out; @out\u{200b} = 1.0;",
        "writes @out; \\x00",
        "/* never closed",
    ];
    for src in attempts {
        let r = outcome(src, DEFAULT_FUEL);
        assert!(r.starts_with("E-"), "`{src}` must be a compile error, got `{r}`");
    }
}

/// N3-S1′: an identifier that is not one of the language's own names is never a function, a variable or an attribute
/// (the language's own functions take numbers and return numbers; none reaches outside).
#[test]
fn only_the_closed_function_list_can_be_called() {
    for name in ["open", "write", "read", "exit", "abort", "kill", "fork", "mmap", "ptrace", "dlopen", "syscall", "log10_file", "ABS", "Sin", "noise2"] {
        assert!(outcome(&format!("writes @out; @out = {name}(1.0);"), DEFAULT_FUEL).starts_with("E-"), "{name}");
    }
    // an attribute that was not declared, an unknown context name and a write to a read-only name
    assert!(outcome("writes @out; @out = @secret;", DEFAULT_FUEL).starts_with("E-"));
    assert!(outcome("reads @density; @density = 1.0;", DEFAULT_FUEL).starts_with("E-"));
    assert!(outcome("writes @out; @out = environ;", DEFAULT_FUEL).starts_with("E-"));
    assert!(outcome("writes @out; P = vec3(1.0);", DEFAULT_FUEL).starts_with("E-"));
}

/// N3-S2′: nothing loops without a compile-time bound, and the bounds themselves are bounded.
#[test]
fn loops_and_sizes_stop_at_their_limits() {
    let s = schema();
    let code = |src: &str| compile(src, &s).map(|_| "ok").unwrap_or_else(|d| d.code);
    assert_eq!(code("writes @out; for i in 0..1025 { @out = 1.0; }"), "E-LOOP-2");
    assert_eq!(code("writes @out; for i in 0..1024 { for j in 0..1024 { for k in 0..2 { @out = 1.0; } } }"), "E-LOOP-3");
    assert!(code("writes @out; for i in 0..@density { @out = 1.0; }").starts_with("E-LOOP"));
    assert!(code("writes @out; float n = 4.0; for i in 0..n { @out = 1.0; }").starts_with("E-"));
    assert_eq!(code(&format!("writes @out; @out = {}1.0{};", "(".repeat(5000), ")".repeat(5000))), "E-LIMIT-2");
    assert_eq!(code(&format!("writes @out; // {}", "x".repeat(cfa4_expr::MAX_SOURCE_BYTES))), "E-LIMIT-1");
    // the largest legal loop product compiles
    assert_eq!(code("writes @out; float a = 0.0; for i in 0..1024 { for j in 0..1024 { a = a + 1.0; } } @out = a;"), "ok");
}

/// N3-S2′: the largest legal program (1 048 576 iterations) does not hang: it stops with `R-FUEL` at the fuel limit,
/// in a time that is measured and bounded (a generous 20 s on any CI machine; the figure is printed).
#[test]
fn the_worst_legal_program_stops_at_the_fuel_limit() {
    let src = "writes @out; float a = 0.0; for i in 0..1024 { for j in 0..1024 { a = a + sin(float(i)) * cos(float(j)); } } @out = a;";
    for fuel in [DEFAULT_FUEL, MAX_FUEL, u32::MAX] {
        let t = Instant::now();
        assert_eq!(outcome(src, fuel), "R-FUEL", "fuel {fuel}");
        let took = t.elapsed();
        println!("fuel {fuel}: stopped after {took:?}");
        assert!(took < Duration::from_secs(20), "fuel {fuel} took {took:?}");
    }
    // the fuel limit is a hard ceiling: asking for more than MAX_FUEL runs no more than MAX_FUEL instructions
    let s = schema();
    let p = compile(src, &s).unwrap();
    let mut vm = p.new_vm();
    let mut out = zeros(&s);
    let err = p.run(&mut vm, u32::MAX, &Env::default(), &[], &zeros(&s), &mut out).unwrap_err();
    assert_eq!(err.code, "R-FUEL");
    // zero fuel stops at once
    assert_eq!(outcome("writes @out; @out = 1.0;", 0), "R-FUEL");
}

/// N3-S2′: int overflow, division by zero and NaN/inf are run-time errors or marked values, never a panic or a trap.
#[test]
fn arithmetic_traps_are_errors() {
    for src in [
        "writes @out; int a = 2147483647; int b = a + 1; @out = float(b);",
        "writes @out; int a = -2147483647 - 1; int b = a - 1; @out = float(b);",
        "writes @out; int a = -2147483647 - 1; int b = -a; @out = float(b);",
        "writes @out; int a = -2147483647 - 1; int b = a / -1; @out = float(b);",
        "writes @out; int a = -2147483647 - 1; int b = a % -1; @out = float(b);",
        "writes @out; int z = 0; @out = float(1 / z);",
        "writes @out; int z = 0; @out = float(1 % z);",
        "writes @out; int a = 65536; @out = float(a * a);",
    ] {
        assert_eq!(outcome(src, DEFAULT_FUEL), "R-INT", "{src}");
    }
    for src in [
        "writes @out; @out = sqrt(-1.0) + log(0.0) + pow(0.0, -1.0) + exp(1000.0);",
        "writes @out; @out = float(int(1e30));",
        "writes @out; @out = 1.0 / 0.0 - 1.0 / 0.0;",
        "writes @out; @out = normalize(vec3(0.0)).x;",
        "writes @out; @out = fit(1.0, 2.0, 2.0, 3.0, 4.0);",
        "writes @out; @out = noise(vec3(1e38, -1e38, 1e38));",
    ] {
        let r = outcome(src, DEFAULT_FUEL);
        assert!(r == "ok" || r.starts_with("R-") || r.starts_with("E-"), "{src}: {r}");
    }
}

fn rss_kib() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    s.lines().find_map(|l| l.strip_prefix("VmRSS:")?.trim().trim_end_matches("kB").trim().parse().ok())
}

/// N3-S2′: compiling and running hostile programs over and over leaves the process' resident memory bounded: the VM's
/// stack and slots are sized at compile time and nothing is allocated per voxel. Measured, not assumed.
#[test]
fn resident_memory_stays_bounded() {
    let s = schema();
    let programs = [
        format!("writes @out; @out = {}1.0{};", "(".repeat(60), ")".repeat(60)),
        "writes @out; float a = 0.0; for i in 0..1024 { for j in 0..1024 { a = a + 1.0; } } @out = a;".to_string(),
        format!("writes @out; {}", (0..250).map(|i| format!("float v{i} = {i}.5; ")).collect::<String>()),
        "writes @vel; @vel = vec3(noise(P), noise(P * 2.0), noise(P * 3.0));".to_string(),
        format!("writes @out; // {}", "x".repeat(60_000)),
    ];
    let compiled: Vec<_> = programs.iter().filter_map(|p| compile(p, &s).ok()).collect();
    assert!(compiled.len() >= 4);
    // warm up, then measure
    let spin = |rounds: usize| {
        for _ in 0..rounds {
            for p in programs.iter() {
                let _ = compile(p, &s);
            }
            for p in &compiled {
                let mut vm = p.new_vm();
                let mut out = zeros(&s);
                let _ = p.run(&mut vm, 5_000, &Env::default(), &[], &zeros(&s), &mut out);
            }
        }
    };
    spin(20);
    let before = rss_kib();
    spin(400);
    let after = rss_kib();
    if let (Some(b), Some(a)) = (before, after) {
        println!("resident memory: {b} kB before, {a} kB after 400 rounds of {} compiles and {} runs", programs.len(), compiled.len());
        assert!(a < b + 64 * 1024, "the process grew from {b} kB to {a} kB");
    }
    // one VM serves a million voxels without growing its buffers
    let p = &compiled[3];
    let mut vm = p.new_vm();
    let mut out = zeros(&s);
    let before = rss_kib();
    for i in 0..1_000_000i32 {
        let env = Env { p: [i as f32, 0.5, 0.25], ijk: [i, 0, 0], ..Env::default() };
        p.run(&mut vm, DEFAULT_FUEL, &env, &[], &zeros(&s), &mut out).unwrap();
    }
    let after = rss_kib();
    if let (Some(b), Some(a)) = (before, after) {
        assert!(a < b + 16 * 1024, "a million voxels grew the process from {b} kB to {a} kB");
    }
}

/// Speed of the interpreter, for the record (plan §7): voxels per second on this machine for a typical and a
/// heavy expression. Printed, with a floor far below any real machine, so the measurement is repeatable with
/// `cargo test -p cfa4-expr --release --test hostile -- --nocapture speed`. The reference-machine figure is the owner's run.
#[test]
fn speed_of_the_vm_is_measured() {
    let s = schema();
    for (name, src, voxels) in [
        ("threshold", "reads @density; writes @out; @out = @density > 0.5 ? 1.0 : 0.0;", 2_000_000u32),
        (
            "noise field",
            "writes @vel; @vel = vec3(noise(P * 0.05), noise(P * 0.05 + vec3(31.4, 0.0, 0.0)), noise(P * 0.05 + vec3(0.0, 17.2, 0.0))) * 2.0;",
            500_000,
        ),
        ("loop of 64", "reads @density; writes @out; float a = 0.0; for i in 0..64 { a = a + sin(float(i) * @density); } @out = a;", 100_000),
    ] {
        let p = compile(src, &s).unwrap();
        let mut vm = p.new_vm();
        let mut out = zeros(&s);
        let input = zeros(&s);
        let t = Instant::now();
        for i in 0..voxels as i32 {
            let env = Env { p: [i as f32, 0.5, 0.25], ijk: [i, 0, 0], ..Env::default() };
            p.run(&mut vm, DEFAULT_FUEL, &env, &[], &input, &mut out).unwrap();
        }
        let took = t.elapsed();
        let per_s = f64::from(voxels) / took.as_secs_f64();
        println!("{name}: {per_s:.0} voxels/s on one thread ({took:?} for {voxels})");
        assert!(per_s > 10_000.0, "{name} is far too slow: {per_s}");
    }
}
