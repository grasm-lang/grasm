// The Rust playground's libm, as a wasm module of its own: `f64`'s methods,
// which rustc builds for `wasm32-unknown-unknown` from the same `libm` code
// it links into `grasm_wasm.wasm`. `scripts/fuzz_wasmmath.sh` builds it with
// `rustc --target wasm32-unknown-unknown --crate-type cdylib -O` and holds
// the wasm C library's functions (`compiler/rtwasm/src/math.gr`) to it, bit
// for bit. Each export is `r_<the C name>`: a function named as the C one
// would be the very symbol it calls.
#[no_mangle] pub extern "C" fn r_sin(x: f64) -> f64 { x.sin() }
#[no_mangle] pub extern "C" fn r_cos(x: f64) -> f64 { x.cos() }
#[no_mangle] pub extern "C" fn r_tan(x: f64) -> f64 { x.tan() }
#[no_mangle] pub extern "C" fn r_exp(x: f64) -> f64 { x.exp() }
#[no_mangle] pub extern "C" fn r_log(x: f64) -> f64 { x.ln() }
#[no_mangle] pub extern "C" fn r_log2(x: f64) -> f64 { x.log2() }
#[no_mangle] pub extern "C" fn r_log10(x: f64) -> f64 { x.log10() }
#[no_mangle] pub extern "C" fn r_pow(x: f64, y: f64) -> f64 { x.powf(y) }
#[no_mangle] pub extern "C" fn r_asin(x: f64) -> f64 { x.asin() }
#[no_mangle] pub extern "C" fn r_acos(x: f64) -> f64 { x.acos() }
#[no_mangle] pub extern "C" fn r_atan(x: f64) -> f64 { x.atan() }
#[no_mangle] pub extern "C" fn r_atan2(y: f64, x: f64) -> f64 { y.atan2(x) }
#[no_mangle] pub extern "C" fn r_sinh(x: f64) -> f64 { x.sinh() }
#[no_mangle] pub extern "C" fn r_cosh(x: f64) -> f64 { x.cosh() }
#[no_mangle] pub extern "C" fn r_tanh(x: f64) -> f64 { x.tanh() }
#[no_mangle] pub extern "C" fn r_cbrt(x: f64) -> f64 { x.cbrt() }
#[no_mangle] pub extern "C" fn r_hypot(x: f64, y: f64) -> f64 { x.hypot(y) }
#[no_mangle] pub extern "C" fn r_fmod(x: f64, y: f64) -> f64 { x % y }
#[no_mangle] pub extern "C" fn r_round(x: f64) -> f64 { x.round() }
#[no_mangle] pub extern "C" fn r_fmin(x: f64, y: f64) -> f64 { x.min(y) }
#[no_mangle] pub extern "C" fn r_fmax(x: f64, y: f64) -> f64 { x.max(y) }
#[no_mangle] pub extern "C" fn r_floor(x: f64) -> f64 { x.floor() }
#[no_mangle] pub extern "C" fn r_ceil(x: f64) -> f64 { x.ceil() }
#[no_mangle] pub extern "C" fn r_sqrt(x: f64) -> f64 { x.sqrt() }
