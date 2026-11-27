use crate::*;

fn schema(ctx: Context) -> Schema {
    Schema {
        context: ctx,
        attrs: vec![
            ("density".into(), Ty::Float),
            ("temperature".into(), Ty::Float),
            ("color".into(), Ty::Vec3),
            ("mask".into(), Ty::Float),
            ("vel".into(), Ty::Vec3),
            ("out".into(), Ty::Float),
            ("n".into(), Ty::Int),
            ("Cd".into(), Ty::Vec3),
            ("flag".into(), Ty::Bool),
        ],
    }
}

fn zeros(s: &Schema) -> Vec<Value> {
    s.attrs.iter().map(|(_, t)| Value::zero(*t)).collect()
}

fn run(src: &str, ctx: Context, set: &[(usize, Value)], env: Env) -> std::result::Result<Vec<Value>, Diagnostic> {
    let s = schema(ctx);
    let p = compile(src, &s)?;
    let mut inputs = zeros(&s);
    for (i, v) in set {
        inputs[*i] = *v;
    }
    let mut outputs = zeros(&s);
    let mut vm = p.new_vm();
    p.run(&mut vm, DEFAULT_FUEL, &env, &[], &inputs, &mut outputs)?;
    Ok(outputs)
}

fn vol(src: &str, set: &[(usize, Value)]) -> Vec<Value> {
    run(src, Context::Volume, set, Env::default()).unwrap_or_else(|e| panic!("{e}"))
}

fn code(src: &str) -> &'static str {
    match run(src, Context::Volume, &[], Env::default()) {
        Ok(_) => "ok",
        Err(e) => e.code,
    }
}

const DENS: usize = 0;
const TEMP: usize = 1;
const COLOR: usize = 2;
const MASK: usize = 3;
const VEL: usize = 4;
const OUT: usize = 5;
const N: usize = 6;
const CD: usize = 7;

#[test]
fn threshold_example() {
    let src = "reads @density; writes @mask; @mask = @density > 0.5 ? 1.0 : 0.0;";
    assert_eq!(vol(src, &[(DENS, Value::Float(0.7))])[MASK], Value::Float(1.0));
    assert_eq!(vol(src, &[(DENS, Value::Float(0.2))])[MASK], Value::Float(0.0));
}

#[test]
fn color_example_with_param_and_fit() {
    let src = "reads @density, @temperature; writes @color; param float gain = 2.0;
        float t = fit(@temperature, 300.0, 1200.0, 0.0, 1.0);
        if (@density > 0.01) { @color = lerp(vec3(0.1, 0.0, 0.0), vec3(1.0, 0.6, 0.1), t) * gain; }
        else { @color = vec3(0.0, 0.0, 0.0); }";
    let o = vol(src, &[(DENS, Value::Float(1.0)), (TEMP, Value::Float(1200.0))]);
    assert_eq!(o[COLOR], Value::Vec3([2.0, 1.2, 0.2]));
    let o = vol(src, &[(DENS, Value::Float(0.0)), (TEMP, Value::Float(1200.0))]);
    assert_eq!(o[COLOR], Value::Vec3([0.0; 3]));
}

#[test]
fn noise_field_example_is_bounded_and_deterministic() {
    let src = "writes @vel; param float scale = 0.05; param float amp = 2.0;
        vec3 q = P * scale;
        @vel = vec3(noise(q), noise(q + vec3(31.4, 0.0, 0.0)), noise(q + vec3(0.0, 17.2, 0.0))) * amp;";
    let env = Env { p: [12.5, -3.25, 100.0], ..Env::default() };
    let a = run(src, Context::Volume, &[], env).unwrap();
    let b = run(src, Context::Volume, &[], env).unwrap();
    assert!(a[VEL].same_bits(&b[VEL]));
    let Value::Vec3(v) = a[VEL] else { panic!() };
    assert!(v.iter().all(|c| c.abs() <= 2.0));
}

#[test]
fn bounded_loop_example() {
    let src = "reads @density; writes @out; float acc = 0.0;
        for i in 0..8 { acc = acc + @density * float(i) * 0.125; }
        @out = acc;";
    let o = vol(src, &[(DENS, Value::Float(1.0))]);
    let Value::Float(f) = o[OUT] else { panic!() };
    assert!((f - 3.5).abs() < 1e-5, "{f}");
}

#[test]
fn points_example() {
    let src = "writes @Cd; float h = fit(P.y, 0.0, 10.0, 0.0, 1.0);
        @Cd = lerp(vec3(0.0, 0.2, 1.0), vec3(1.0, 0.3, 0.0), clamp(h, 0.0, 1.0));";
    let o = run(src, Context::Points, &[], Env { p: [0.0, 10.0, 0.0], ..Env::default() }).unwrap();
    assert_eq!(o[CD], Value::Vec3([1.0, 0.3, 0.0]));
}

#[test]
fn points_id_and_volume_names() {
    let o = run("writes @n; @n = id * 2;", Context::Points, &[], Env { id: 21, ..Env::default() }).unwrap();
    assert_eq!(o[N], Value::Int(42));
    assert_eq!(code("writes @n; @n = id;"), "E-ATTR-4");
    let r = run("writes @out; @out = voxelsize;", Context::Points, &[], Env::default());
    assert_eq!(r.unwrap_err().code, "E-ATTR-4");
    let o = run("writes @out; @out = voxelsize + ijk.y;", Context::Volume, &[], Env { voxel_size: 0.5, ijk: [1, 2, 3], ..Env::default() }).unwrap();
    assert_eq!(o[OUT], Value::Float(2.5));
}

#[test]
fn read_sees_value_before_the_run() {
    let o = vol("reads @out; writes @out; @out = @out + 1.0; @out = @out + 1.0;", &[(OUT, Value::Float(5.0))]);
    assert_eq!(o[OUT], Value::Float(6.0));
}

#[test]
fn rejected_programs_have_stable_codes() {
    assert_eq!(code("@mask = 1.0;"), "E-ATTR-3");
    assert_eq!(code("writes @mask; @mask = @density;"), "E-ATTR-2");
    assert_eq!(code("reads @nope;"), "E-ATTR-1");
    assert_eq!(code("writes @out; for i in 0..n { }"), "E-LOOP-1");
    assert_eq!(code("writes @out; for i in 0..0 { }"), "E-LOOP-2");
    assert_eq!(code("writes @out; for i in 0..1025 { }"), "E-LOOP-2");
    assert_eq!(code("writes @out; for i in 0..1000 { for j in 0..1000 { for k in 0..2 { } } }"), "E-LOOP-3");
    assert_eq!(code("open(\"/etc/passwd\");"), "E-LEX-1");
    assert_eq!(code("writes @out; @out = open(1.0);"), "E-TYPE-3");
    assert_eq!(code("writes @out; @out = system(1);"), "E-TYPE-3");
    assert_eq!(code("writes @out; @out = sin(1.0, 2.0);"), "E-TYPE-4");
    assert_eq!(code("writes @out; @out = true;"), "E-TYPE-1");
    assert_eq!(code("writes @out; float x = 1.0; float x = 2.0;"), "E-TYPE-5");
    assert_eq!(code("writes @out; for i in 0..3 { i = 2; }"), "E-TYPE-6");
    assert_eq!(code("writes @out; param float g = 1.0; g = 2.0;"), "E-TYPE-6");
    assert_eq!(code("writes @out; @out = unknown;"), "E-TYPE-2");
    assert_eq!(code("writes @out; @out = 1.0 +;"), "E-PARSE-1");
    assert_eq!(code("writes @out; @out = 1.0; reads @density;"), "E-PARSE-2");
    assert_eq!(code("/* open"), "E-LEX-2");
    assert_eq!(code("writes @out; @out = 99999999999;"), "E-LEX-3");
    assert_eq!(code("writes @out; while (true) { }"), "E-PARSE-1");
}

#[test]
fn error_position_is_exact() {
    let s = schema(Context::Volume);
    let e = compile("writes @out;\n@out = 1.0 +;\n", &s).unwrap_err();
    assert_eq!((e.code, e.line, e.col), ("E-PARSE-1", 2, 13));
    let e = compile("writes @out;\n  @out = foo(1.0);", &s).unwrap_err();
    assert_eq!((e.code, e.line, e.col), ("E-TYPE-3", 2, 10));
}

#[test]
fn integer_literal_becomes_float_with_a_warning() {
    let s = schema(Context::Volume);
    let p = compile("writes @out; float x = 1; @out = x * 2;", &s).unwrap();
    assert_eq!(p.warnings().len(), 2);
    assert!(p.warnings().iter().all(|w| w.code == "W-TYPE-1"));
    assert_eq!(vol("writes @out; @out = -3;", &[])[OUT], Value::Float(-3.0));
    assert_eq!(code("writes @out; int i = 2; @out = i;"), "E-TYPE-1");
}

#[test]
fn integer_errors_are_run_time_errors_with_a_place() {
    let s = schema(Context::Volume);
    let p = compile("writes @n; int a = 2147483647; @n = a + 1;", &s).unwrap();
    let mut vm = p.new_vm();
    let mut out = zeros(&s);
    let e = p.run(&mut vm, DEFAULT_FUEL, &Env::default(), &[], &zeros(&s), &mut out).unwrap_err();
    assert_eq!((e.code, e.line), ("R-INT", 1));
    assert_eq!(code("writes @n; int z = 0; @n = 5 / z;"), "R-INT");
    assert_eq!(code("writes @n; int z = 0; @n = 5 % z;"), "R-INT");
    assert_eq!(code("writes @n; int m = -2147483647 - 1; @n = abs(m);"), "R-INT");
}

#[test]
fn fuel_stops_long_programs() {
    let s = schema(Context::Volume);
    let src = "writes @out; float a = 0.0; for i in 0..1024 { for j in 0..1024 { a = a + 1.0; } } @out = a;";
    let p = compile(src, &s).unwrap();
    let mut vm = p.new_vm();
    let mut out = zeros(&s);
    let e = p.run(&mut vm, 5_000, &Env::default(), &[], &zeros(&s), &mut out).unwrap_err();
    assert_eq!(e.code, "R-FUEL");
    // The machine is reusable after an error.
    let p2 = compile("writes @out; @out = 1.0;", &s).unwrap();
    let mut vm2 = p2.new_vm();
    assert!(p2.run(&mut vm2, 100, &Env::default(), &[], &zeros(&s), &mut out).is_ok());
    // A huge request is capped.
    let e = p.run(&mut vm, u32::MAX, &Env::default(), &[], &zeros(&s), &mut out).unwrap_err();
    assert_eq!(e.code, "R-FUEL");
}

#[test]
fn undefined_cases_follow_the_table() {
    let nan = |src: &str| matches!(vol(src, &[])[OUT], Value::Float(f) if f.is_nan());
    assert!(nan("writes @out; @out = sqrt(-1.0);"));
    assert!(nan("writes @out; @out = log(0.0);"));
    assert!(nan("writes @out; @out = log(-3.0);"));
    assert_eq!(vol("writes @out; @out = pow(0.0, -1.0);", &[])[OUT], Value::Float(f32::INFINITY));
    assert_eq!(vol("writes @color; @color = normalize(vec3(0.0, 0.0, 0.0));", &[])[COLOR], Value::Vec3([0.0; 3]));
    assert_eq!(vol("writes @out; @out = atan2(0.0, 0.0);", &[])[OUT], Value::Float(0.0));
    assert_eq!(vol("writes @out; @out = fit(5.0, 1.0, 1.0, 7.0, 9.0);", &[])[OUT], Value::Float(7.0));
    assert_eq!(vol("writes @out; @out = fit(50.0, 0.0, 10.0, 0.0, 1.0);", &[])[OUT], Value::Float(1.0));
    assert_eq!(vol("writes @n; @n = int(1.0 / 0.0);", &[])[N], Value::Int(i32::MAX));
    assert_eq!(vol("writes @n; @n = int(sqrt(-1.0));", &[])[N], Value::Int(0));
    assert_eq!(vol("writes @n; @n = int(-2.9);", &[])[N], Value::Int(-2));
}

#[test]
fn nan_output_is_canonical_and_reported() {
    let s = schema(Context::Volume);
    let p = compile("writes @out; @out = sqrt(-1.0);", &s).unwrap();
    let mut vm = p.new_vm();
    let mut out = zeros(&s);
    let st = p.run(&mut vm, 100, &Env::default(), &[], &zeros(&s), &mut out).unwrap();
    assert!(st.wrote_nan);
    assert!(out[OUT].same_bits(&Value::Float(f32::NAN)));
}

#[test]
fn operators_and_precedence() {
    let f = |src: &str| match vol(&format!("writes @out; @out = {src};"), &[])[OUT] {
        Value::Float(v) => v,
        o => panic!("{o:?}"),
    };
    assert_eq!(f("1.0 + 2.0 * 3.0"), 7.0);
    assert_eq!(f("(1.0 + 2.0) * 3.0"), 9.0);
    assert_eq!(f("10.0 - 4.0 - 3.0"), 3.0);
    assert_eq!(f("7.0 % 4.0"), 3.0);
    assert_eq!(f("1.0 < 2.0 && 2.0 < 3.0 ? 5.0 : 6.0"), 5.0);
    assert_eq!(f("1.0 > 2.0 || 2.0 < 3.0 ? 5.0 : 6.0"), 5.0);
    assert_eq!(f("!(1.0 > 2.0) ? 5.0 : 6.0"), 5.0);
    assert_eq!(f("true ? false ? 1.0 : 2.0 : 3.0"), 2.0);
    assert_eq!(f("2.0 * 3 + 1"), 7.0);
    assert_eq!(f("vec3(1.0, 2.0, 3.0).y"), 2.0);
    assert_eq!(f("dot(vec3(1.0, 2.0, 3.0), vec3(1.0, 1.0, 1.0))"), 6.0);
    assert_eq!(f("length(vec3(3.0, 4.0, 0.0))"), 5.0);
    assert_eq!(f("smoothstep(0.0, 1.0, 0.5)"), 0.5);
    assert_eq!(f("step(0.5, 0.4)"), 0.0);
    assert_eq!(f("clamp(5.0, 0.0, 1.0)"), 1.0);
    assert_eq!(f("min(1.0, 2.0) + max(1.0, 2.0)"), 3.0);
    assert_eq!(f("round(2.5)"), 3.0);
    assert_eq!(f("floor(-0.5) + ceil(0.2)"), 0.0);
    assert_eq!(f("rand(7, 3)").clamp(0.0, 1.0), f("rand(7, 3)"));
    assert_eq!(f("float(hash(1, 2, 3) == hash(1, 2, 3) ? 1 : 0)"), 1.0);
}

#[test]
fn short_circuit_skips_the_unused_side() {
    // `1 / z` would be a run-time error; the right side is never evaluated.
    assert_eq!(code("writes @out; int z = 0; @out = (z != 0 && 1 / z > 0) ? 1.0 : 0.0;"), "ok");
    assert_eq!(code("writes @out; int z = 0; @out = (z == 0 || 1 / z > 0) ? 1.0 : 0.0;"), "ok");
    assert_eq!(code("writes @out; int z = 0; @out = z == 0 ? 0.0 : float(1 / z);"), "ok");
}

#[test]
fn params_use_defaults_and_overrides() {
    let s = schema(Context::Volume);
    let p = compile("writes @out; param float g = 3; param vec3 v = vec3(1.0, 2.0, 3.0); @out = g + v.z;", &s).unwrap();
    assert_eq!(p.params().len(), 2);
    assert_eq!(p.params()[0].default, Value::Float(3.0));
    let mut vm = p.new_vm();
    let mut out = zeros(&s);
    p.run(&mut vm, 100, &Env::default(), &[], &zeros(&s), &mut out).unwrap();
    assert_eq!(out[OUT], Value::Float(6.0));
    p.run(&mut vm, 100, &Env::default(), &[Value::Float(10.0)], &zeros(&s), &mut out).unwrap();
    assert_eq!(out[OUT], Value::Float(13.0));
    let e = p.run(&mut vm, 100, &Env::default(), &[Value::Int(1)], &zeros(&s), &mut out).unwrap_err();
    assert_eq!(e.code, "R-PARAM");
}

#[test]
fn noise_properties() {
    let mut sum = 0.0f64;
    let mut max = 0.0f32;
    let mut cnt = 0;
    for ix in 0..20 {
        for iy in 0..20 {
            for iz in 0..20 {
                let v = crate::vm::noise3([ix as f32 * 0.37 + 0.11, iy as f32 * 0.41 + 0.23, iz as f32 * 0.29 + 0.31]);
                assert!((-1.0..=1.0).contains(&v));
                sum += f64::from(v);
                max = max.max(v.abs());
                cnt += 1;
            }
        }
    }
    assert!((sum / f64::from(cnt)).abs() < 0.1, "mean {}", sum / f64::from(cnt));
    assert!(max > 0.2, "noise is not flat");
    // Zero at lattice points, smooth (continuous) nearby, defined for non-finite input.
    assert_eq!(crate::vm::noise3([3.0, -2.0, 5.0]), 0.0);
    assert!((crate::vm::noise3([1.5, 1.5, 1.5]) - crate::vm::noise3([1.5001, 1.5, 1.5])).abs() < 0.01);
    assert_eq!(crate::vm::noise3([f32::NAN, 0.0, 0.0]), 0.0);
    assert!(crate::vm::noise3([1e30, 1e30, 1e30]).abs() <= 1.0);
}

#[test]
fn limits_do_not_overflow_the_stack() {
    let s = schema(Context::Volume);
    let deep = format!("writes @out; @out = {}1.0{};", "(".repeat(30_000), ")".repeat(30_000));
    assert_eq!(compile(&deep, &s).unwrap_err().code, "E-LIMIT-2");
    let neg = format!("writes @out; @out = {}1.0;", "-".repeat(50_000));
    assert_eq!(compile(&neg, &s).unwrap_err().code, "E-LIMIT-2");
    let long_chain = format!("writes @out; @out = 1.0{};", " + 1.0".repeat(10_000));
    assert_eq!(compile(&long_chain, &s).unwrap_err().code, "E-LIMIT-2");
    let blocks = format!("writes @out; {}{}", "{".repeat(10_000), "}".repeat(10_000));
    assert_eq!(compile(&blocks, &s).unwrap_err().code, "E-LIMIT-2");
    let ifs = format!("writes @out; {} {}", "if (true) { } else ".repeat(3_000), "{ }");
    assert_eq!(compile(&ifs, &s).unwrap_err().code, "E-LIMIT-2");
    let big = format!("writes @out; // {}", "x".repeat(70_000));
    assert_eq!(compile(&big, &s).unwrap_err().code, "E-LIMIT-1");
    let many = format!("writes @out; {}", (0..300).map(|i| format!("float v{i} = 1.0; ")).collect::<String>());
    assert_eq!(compile(&many, &s).unwrap_err().code, "E-LIMIT-3");
    let ident = format!("writes @out; float {} = 1.0;", "a".repeat(70));
    assert_eq!(compile(&ident, &s).unwrap_err().code, "E-LEX-4");
}

#[test]
fn deepest_legal_expression_compiles_and_runs() {
    let s = schema(Context::Volume);
    let src = format!("writes @out; @out = {}1.0{};", "(".repeat(30), ")".repeat(30));
    assert!(compile(&src, &s).is_ok());
    let chain = format!("writes @out; @out = 1.0{};", " + 1.0".repeat(60));
    let p = compile(&chain, &s).unwrap();
    let mut vm = p.new_vm();
    let mut out = zeros(&s);
    p.run(&mut vm, 1000, &Env::default(), &[], &zeros(&s), &mut out).unwrap();
    assert_eq!(out[OUT], Value::Float(61.0));
}

#[test]
fn results_do_not_depend_on_threads() {
    let s = schema(Context::Volume);
    let p = compile("reads @density; writes @out; @out = noise(P * 0.1) * @density + rand(3, int(ijk.x));", &s).unwrap();
    let eval = |range: std::ops::Range<i32>| -> Vec<u32> {
        let mut vm = p.new_vm();
        let mut res = Vec::new();
        for i in range {
            let env = Env { p: [i as f32 * 0.7, 0.3, -1.1], ijk: [i, 0, 0], ..Env::default() };
            let mut inp = zeros(&s);
            inp[DENS] = Value::Float(0.5);
            let mut out = zeros(&s);
            p.run(&mut vm, 1000, &env, &[], &inp, &mut out).unwrap();
            let Value::Float(f) = out[OUT] else { panic!() };
            res.push(f.to_bits());
        }
        res
    };
    let whole = eval(0..400);
    let mut parts = Vec::new();
    std::thread::scope(|sc| {
        let hs: Vec<_> = (0..4).map(|k| sc.spawn(move || (k, eval(k * 100..k * 100 + 100)))).collect();
        for h in hs {
            parts.push(h.join().unwrap());
        }
    });
    parts.sort_by_key(|(k, _)| *k);
    let joined: Vec<u32> = parts.into_iter().flat_map(|(_, v)| v).collect();
    assert_eq!(whole, joined);
}

/// Deterministic garbage and mutated programs must never panic.
#[test]
fn hostile_inputs_never_panic() {
    let s = schema(Context::Volume);
    let seeds = [
        "reads @density; writes @mask; @mask = @density > 0.5 ? 1.0 : 0.0;",
        "writes @out; float a = 0.0; for i in 0..8 { a = a + noise(vec3(float(i))); } @out = a;",
        "writes @vel; param float s = 1.0; @vel = vec3(1.0) * s + P;",
    ];
    let mut x = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let mut compiled = 0;
    for round in 0..4000 {
        let mut b: Vec<u8> =
            if round % 4 == 0 { (0..(next() % 80)).map(|_| (next() & 0x7f) as u8).collect() } else { seeds[round % seeds.len()].as_bytes().to_vec() };
        for _ in 0..(next() % 4) {
            if b.is_empty() {
                break;
            }
            let i = (next() as usize) % b.len();
            match next() % 3 {
                0 => b[i] = (next() & 0x7f) as u8,
                1 => {
                    b.remove(i);
                }
                _ => {
                    let pool = b"(){};@=+-*/.,<>!?:&|0123456789xyz ";
                    b.insert(i, pool[(next() as usize) % pool.len()])
                }
            }
        }
        let src = String::from_utf8_lossy(&b).into_owned();
        if let Ok(p) = compile(&src, &s) {
            compiled += 1;
            let mut vm = p.new_vm();
            let mut out = zeros(&s);
            let _ = p.run(&mut vm, 2000, &Env::default(), &[], &zeros(&s), &mut out);
        }
    }
    assert!(compiled > 100, "the mutations should leave some programs valid ({compiled})");
}
