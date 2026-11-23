//! Behaviour tests of the expression engine: the language, determinism, host reads and the work
//! limits that keep a hostile expression from hanging or crashing the process.

use std::collections::HashMap;

use cfa4_time::Tick;

use super::*;

/// A host with a few properties: `a` (keys 0 s: 0, 1 s: 10, 2 s: 20), `v` (a vector) and `loop`.
struct Props {
    keys: HashMap<&'static str, (Vec<f64>, Vec<f64>)>,
}

impl Props {
    fn new() -> Props {
        let mut keys = HashMap::new();
        keys.insert("a", (vec![0.0, 1.0, 2.0], vec![0.0, 10.0, 20.0]));
        Props { keys }
    }
}

impl Host for Props {
    fn key_info(&self, path: &str) -> Option<KeyInfo> {
        let (t, v) = self.keys.get(path)?;
        Some(KeyInfo { times: t.clone(), values: v.iter().map(|x| Value::Num(*x)).collect() })
    }
    fn value_at(&self, path: &str, time: f64) -> Option<Value> {
        let (t, v) = self.keys.get(path)?;
        // Linear between keys, held outside them.
        let i = t.iter().rposition(|k| *k <= time).unwrap_or(0);
        let j = (i + 1).min(t.len() - 1);
        let span = t[j] - t[i];
        let u = if span > 0.0 { ((time - t[i]) / span).clamp(0.0, 1.0) } else { 0.0 };
        Some(Value::Num(v[i] + (v[j] - v[i]) * u))
    }
}

fn eval_at(text: &str, secs: f64, value: Value, host: &dyn Host) -> Result<Value, ExprError> {
    evaluate(&Evaluation { text, time: Tick::from_seconds_f64(secs), value, property: "p", property_uid: 7, layer_index: 3, frame_duration: 1.0 / 30.0, host })
}

fn run(text: &str) -> Result<Value, ExprError> {
    eval_at(text, 1.0, Value::Num(5.0), &NoHost)
}

fn num(v: Result<Value, ExprError>) -> f64 {
    match v {
        Ok(Value::Num(x)) => x,
        other => f64::NAN * if other.is_err() { 1.0 } else { 0.0 },
    }
}

#[test]
fn array_maths_works_on_vectors_and_keeps_string_concatenation() {
    assert_eq!(run("[10, 20] + [1, 2]"), Ok(Value::Vec(vec![11.0, 22.0])));
    assert_eq!(run("[1, 2] * 3"), Ok(Value::Vec(vec![3.0, 6.0])));
    assert_eq!(run("value + [1, 1]"), Ok(Value::Vec(vec![6.0, 6.0])));
    assert_eq!(run("'a' + 'b'"), Ok(Value::Text("ab".into())));
    assert_eq!(run("var x = 2; x * 4"), Ok(Value::Num(8.0)));
}

#[test]
fn time_and_value_are_the_evaluation_inputs() {
    assert_eq!(num(eval_at("time", 2.5, Value::Num(0.0), &NoHost)), 2.5);
    assert_eq!(num(eval_at("value * 2", 0.0, Value::Num(4.0), &NoHost)), 8.0);
    assert_eq!(num(eval_at("index", 0.0, Value::Num(0.0), &NoHost)), 3.0);
}

#[test]
fn interpolation_helpers_stay_between_their_ends() {
    for t in [-1.0, 0.0, 0.25, 0.5, 0.75, 1.0, 2.0] {
        let v = num(eval_at(&format!("ease({t}, 0, 1, 10, 20)"), 0.0, Value::Num(0.0), &NoHost));
        assert!((10.0..=20.0).contains(&v), "{t}: {v}");
    }
    assert_eq!(num(run("linear(0.5, 0, 1, 10, 20)")), 15.0);
}

#[test]
fn host_reads_resolve_and_a_missing_property_is_an_error() {
    let host = Props::new();
    assert_eq!(num(eval_at("prop('a').valueAtTime(1.5)", 0.0, Value::Num(0.0), &host)), 15.0);
    assert_eq!(num(eval_at("prop('a').numKeys", 0.0, Value::Num(0.0), &host)), 3.0);
    assert_eq!(num(eval_at("prop('a').key(2).time", 0.0, Value::Num(0.0), &host)), 1.0);
    let e = eval_at("prop('nope').valueAtTime(0)", 0.0, Value::Num(0.0), &host).unwrap_err();
    assert!(e.0.contains("nope"), "{e}");
}

#[test]
fn loop_out_continues_the_motion_past_the_last_key() {
    let host = Props::new();
    // Property `p` reads its own keys through `prop('a')`; loop on `a` directly.
    let cycle = num(eval_at("prop('a').loopOut('cycle')", 2.5, Value::Num(20.0), &host));
    assert_eq!(cycle, 5.0);
    let offset = num(eval_at("prop('a').loopOut('offset')", 2.5, Value::Num(20.0), &host));
    assert!(offset > 20.0);
}

#[test]
fn wiggle_and_random_are_deterministic_and_bounded() {
    let a = num(eval_at("wiggle(2, 30)", 1.7, Value::Num(100.0), &NoHost));
    let b = num(eval_at("wiggle(2, 30)", 1.7, Value::Num(100.0), &NoHost));
    assert_eq!(a, b);
    assert!((a - 100.0).abs() <= 30.0);
    // Another time gives another value; evaluation order does not matter.
    let c = num(eval_at("wiggle(2, 30)", 2.9, Value::Num(100.0), &NoHost));
    assert_ne!(a, c);
    let again = num(eval_at("wiggle(2, 30)", 1.7, Value::Num(100.0), &NoHost));
    assert_eq!(a, again);

    let r1 = num(run("seedRandom(5, true); random(100)"));
    let r2 = num(eval_at("seedRandom(5, true); random(100)", 9.0, Value::Num(5.0), &NoHost));
    assert_eq!(r1, r2, "a timeless seed ignores the time");
    assert!((0.0..100.0).contains(&r1));
}

#[test]
fn a_result_that_is_not_a_finite_number_is_an_error() {
    assert!(run("1 / 0").is_err());
    assert!(run("[1, NaN]").is_err());
    assert!(run("undefined").is_err());
    assert!(run("({a: 1})").is_err());
}

#[test]
fn syntax_errors_name_the_line() {
    let e = run("var x = 1;\nx +* 2").unwrap_err();
    assert!(e.0.starts_with("Error at line 2"), "{e}");
    assert!(check_syntax("1 +").is_err());
    assert!(check_syntax("value + 1").is_ok());
}

// ---- limits: none of these may hang or abort the process -------------------------------------

#[test]
fn an_endless_loop_stops_at_the_work_limit() {
    let e = run("while (true) {}").unwrap_err();
    assert!(!e.0.is_empty());
    // The engine keeps working afterwards (the abandoned context was replaced).
    assert_eq!(run("1 + 1"), Ok(Value::Num(2.0)));
}

#[test]
fn a_long_loop_that_reads_many_values_stops() {
    let host = Props::new();
    let e = eval_at("var s = 0; for (var i = 0; i < 900000; i++) { s += prop('a').valueAtTime(i % 3) } s", 0.0, Value::Num(0.0), &host);
    assert!(e.is_err());
}

#[test]
fn deeply_nested_values_are_refused_not_crashed() {
    let nested = "var a = []; for (var i = 0; i < 5000; i++) a = [a]; String(a).length";
    assert!(run(nested).is_err());
    let json = "var a = []; for (var i = 0; i < 5000; i++) a = [a]; JSON.stringify(a).length";
    assert!(run(json).is_err());
    assert!(run(&format!("{}1{}", "(".repeat(5000), ")".repeat(5000))).is_err());
    assert!(run("eval('(' .repeat(5000) + '1' + ')'.repeat(5000))").is_err());
}

#[test]
fn an_expression_that_reads_itself_is_a_cycle_error() {
    struct Selfish;
    impl Host for Selfish {
        fn key_info(&self, _: &str) -> Option<KeyInfo> {
            Some(KeyInfo::default())
        }
        fn value_at(&self, _: &str, time: f64) -> Option<Value> {
            // The property's own expression again, at the same time.
            eval_at("prop('p').valueAtTime(time + 1)", time, Value::Num(0.0), &Selfish).ok()
        }
    }
    // Each level asks one second later, so there is no exact repeat: the depth limit ends it.
    assert!(eval_at("prop('p').valueAtTime(time + 1)", 0.0, Value::Num(0.0), &Selfish).is_err());
}

#[test]
fn prelude_functions_cannot_be_replaced_by_one_expression_for_the_next() {
    let _ = run("linear = function () { return 99 }; Math.sin = 0; 1");
    assert_eq!(num(run("linear(0.5, 0, 1, 10, 20)")), 15.0);
    assert_eq!(num(run("Math.sin(0)")), 0.0);
}

#[test]
fn no_clock_file_or_network_surface_is_reachable() {
    // The engine is built without `Intl` and without a host clock: `Date` is the only time
    // source JS has, and it must not decide an expression's value across runs.
    let a = run("typeof require + typeof process + typeof fetch + typeof XMLHttpRequest");
    assert_eq!(a, Ok(Value::Text("undefinedundefinedundefinedundefined".into())));
}

#[test]
fn hostile_inputs_never_panic() {
    let seeds = [
        "",
        ";",
        "(",
        ")",
        "[",
        "]",
        "{",
        "}",
        "`${",
        "'",
        "\"",
        "/*",
        "//",
        "a+++b",
        "a---b",
        "1e999",
        "-1e999",
        "0/0",
        "[...Array(1e9)]",
        "new Array(1e10).join()",
        "'x'.repeat(1e9)",
        "Array(1e8).fill(1).map(x => x)",
        "wiggle(1e300, 1e300, 1e9)",
        "noise(1e300)",
        "random(1e300)",
        "loopOut('pingpong', 1e9)",
        "valueAtTime(1e300)",
        "smooth(1e9, 1e9)",
        "key(1e300)",
        "(function f(){ return f() })()",
        "var o = {}; o.o = o; JSON.stringify(o)",
        "thisProperty.wiggle(NaN, NaN)",
        "posterizeTime(1e-300)",
        "posterizeTime(1e300)",
        "timeToFrames(1e300, 1e300)",
    ];
    for s in seeds {
        let _ = eval_at(s, 1e9, Value::Vec(vec![f64::NAN, f64::INFINITY]), &Props::new());
        let _ = check_syntax(s);
    }
    // The engine still answers.
    assert_eq!(run("2 + 2"), Ok(Value::Num(4.0)));
}

#[test]
fn size_taking_constructors_refuse_huge_sizes() {
    for s in ["Array(1e9).length", "new Array(1e10).length", "[].constructor(1e9).length", "'x'.repeat(1e9).length", "'x'.padStart(1e9).length"] {
        assert!(run(s).is_err(), "{s}");
    }
    assert_eq!(run("Array(3).length"), Ok(Value::Num(3.0)));
    assert_eq!(run("'ab'.repeat(3)"), Ok(Value::Text("ababab".into())));
    assert_eq!(run("Array.isArray([1]) ? 1 : 0"), Ok(Value::Num(1.0)));
}
