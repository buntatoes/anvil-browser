// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Value tests: units, colors, math functions, substitution functions,
//! global keywords, and the property table. Written from CSS Values and
//! Units Level 4, CSS Color Level 4/5, and the property definitions.

use anvil_css::parser::parse_component_values;
use anvil_css::properties::{PROPERTIES, is_inherited, property};
use anvil_css::values::{
    Calc, CalcType, Color, GlobalKeyword, Length, LengthUnit, NAMED_COLORS, ResolveContext,
    SystemColor, parse_color_value, parse_hex_color, parse_math,
};
use anvil_css::{Limits, Options, PropertyValue, Value, parse_value};

fn one(property: &str, src: &str) -> Value {
    match parse_value(property, src) {
        Ok(PropertyValue::Values(mut v)) if v.len() == 1 => v.remove(0),
        other => panic!("{property}: {src:?} gave {other:?}"),
    }
}

fn rgba8(src: &str) -> [u8; 4] {
    match one("color", src) {
        Value::Color(Color::Rgba(c)) => c.to_rgba8(),
        other => panic!("{src:?} gave {other:?}"),
    }
}

fn px(property: &str, src: &str) -> f64 {
    let v = one(property, src);
    v.resolve(&ResolveContext::default())
        .unwrap_or_else(|| panic!("{src:?} gave {v:?}"))
}

fn calc(src: &str) -> Calc {
    let v = parse_component_values(src, &Options::default()).unwrap();
    parse_math(&v[0], &Limits::default()).unwrap_or_else(|e| panic!("{src:?}: {e:?}"))
}

fn calc_err(src: &str) -> bool {
    let v = parse_component_values(src, &Options::default()).unwrap();
    parse_math(&v[0], &Limits::default()).is_err()
}

macro_rules! color_cases {
    ($($name:ident: $src:expr => $want:expr;)*) => {
        $(
            #[test]
            fn $name() {
                assert_eq!(rgba8($src), $want, "input {:?}", $src);
            }
        )*
    };
}

color_cases! {
    c_named: "red" => [255, 0, 0, 255];
    c_named_case: "RebeccaPurple" => [102, 51, 153, 255];
    c_transparent: "transparent" => [0, 0, 0, 0];
    c_hex3: "#f80" => [255, 136, 0, 255];
    c_hex4: "#f808" => [255, 136, 0, 136];
    c_hex6: "#123456" => [0x12, 0x34, 0x56, 255];
    c_hex8: "#12345678" => [0x12, 0x34, 0x56, 0x78];
    c_hex_upper: "#ABCDEF" => [0xab, 0xcd, 0xef, 255];
    c_rgb_legacy: "rgb(1, 2, 3)" => [1, 2, 3, 255];
    c_rgba_legacy: "rgba(1, 2, 3, 0.5)" => [1, 2, 3, 128];
    c_rgb_modern: "rgb(1 2 3 / 50%)" => [1, 2, 3, 128];
    c_rgb_percent: "rgb(100% 0% 50%)" => [255, 0, 128, 255];
    c_rgb_clamped: "rgb(300 -5 0)" => [255, 0, 0, 255];
    c_rgb_none: "rgb(none 10 20)" => [0, 10, 20, 255];
    c_rgb_calc: "rgb(calc(100 + 55) 0 0)" => [155, 0, 0, 255];
    c_hsl: "hsl(120 100% 50%)" => [0, 255, 0, 255];
    c_hsl_legacy: "hsla(240, 100%, 50%, 0.5)" => [0, 0, 255, 128];
    c_hsl_angle_unit: "hsl(0.5turn 100% 50%)" => [0, 255, 255, 255];
    c_hsl_negative_hue: "hsl(-120 100% 50%)" => [0, 0, 255, 255];
    c_hwb: "hwb(0 0% 0%)" => [255, 0, 0, 255];
    c_hwb_gray: "hwb(0 50% 50%)" => [128, 128, 128, 255];
    c_lab_white: "lab(100 0 0)" => [255, 255, 255, 255];
    c_lab_black: "lab(0 0 0)" => [0, 0, 0, 255];
    c_lch_white: "lch(100% 0 0)" => [255, 255, 255, 255];
    c_oklab_white: "oklab(1 0 0)" => [255, 255, 255, 255];
    c_oklch_black: "oklch(0 0 0)" => [0, 0, 0, 255];
    c_oklch_alpha: "oklch(1 0 0 / 0.25)" => [255, 255, 255, 64];
    c_color_srgb: "color(srgb 1 0 0)" => [255, 0, 0, 255];
    c_color_srgb_linear: "color(srgb-linear 1 1 1)" => [255, 255, 255, 255];
    c_color_display_p3: "color(display-p3 0 0 0)" => [0, 0, 0, 255];
    c_color_xyz_white: "color(xyz-d65 0.9505 1 1.089)" => [255, 255, 255, 255];
    c_color_mix_srgb: "color-mix(in srgb, red 50%, blue 50%)" => [128, 0, 128, 255];
    c_color_mix_default_50: "color-mix(in srgb, white, black)" => [128, 128, 128, 255];
    c_light_dark: "light-dark(#ff0000, #0000ff)" => [255, 0, 0, 255];
}

macro_rules! bad_color_cases {
    ($($name:ident: $src:expr;)*) => {
        $(
            #[test]
            fn $name() {
                assert!(parse_value("color", $src).is_err(), "{:?} should fail", $src);
            }
        )*
    };
}

bad_color_cases! {
    bc_hex5: "#12345";
    bc_hex_bad_digit: "#12g";
    bc_unknown_name: "reddish";
    bc_mixed_legacy_modern: "rgb(1, 2 3)";
    bc_legacy_none: "rgb(none, 2, 3)";
    bc_too_few: "rgb(1 2)";
    bc_too_many: "rgb(1 2 3 4)";
    bc_unknown_space: "color(nope 1 1 1)";
    bc_unknown_function: "rgbb(1 2 3)";
    bc_length_in_rgb: "rgb(1px 2 3)";
    bc_two_colors: "red blue";
}

#[test]
fn current_color_keyword() {
    assert_eq!(
        one("color", "currentColor"),
        Value::Color(Color::CurrentColor)
    );
}

#[test]
fn system_color_keyword() {
    assert_eq!(
        one("color", "CanvasText"),
        Value::Color(Color::System(SystemColor::CanvasText))
    );
}

#[test]
fn named_color_table_complete() {
    assert_eq!(NAMED_COLORS.len(), 148);
    assert!(NAMED_COLORS.iter().any(|(n, _)| *n == "rebeccapurple"));
    for (name, _) in NAMED_COLORS {
        assert!(parse_value("color", name).is_ok(), "{name}");
    }
}

#[test]
fn hex_helper() {
    assert!(parse_hex_color("fff").is_some());
    assert!(parse_hex_color("ffff").is_some());
    assert!(parse_hex_color("fffff").is_none());
}

#[test]
fn color_value_helper() {
    let v = parse_component_values("hsl(0 100% 50%)", &Options::default()).unwrap();
    assert!(parse_color_value(&v[0], &Limits::default()).is_ok());
}

// Lengths and units.

macro_rules! px_cases {
    ($($name:ident: $prop:expr, $src:expr => $want:expr;)*) => {
        $(
            #[test]
            fn $name() {
                let got = px($prop, $src);
                assert!((got - $want).abs() < 1e-6, "{:?} gave {got}, want {}", $src, $want);
            }
        )*
    };
}

px_cases! {
    l_px: "width", "10px" => 10.0;
    l_in: "width", "1in" => 96.0;
    l_cm: "width", "2.54cm" => 96.0;
    l_mm: "width", "10mm" => 96.0 / 2.54;
    l_q: "width", "40Q" => 96.0 / 2.54;
    l_pt: "width", "72pt" => 96.0;
    l_pc: "width", "1pc" => 16.0;
    l_em: "width", "2em" => 32.0;
    l_rem: "width", "1.5rem" => 24.0;
    l_ex_approx: "width", "1ex" => 8.0;
    l_ch_approx: "width", "2ch" => 16.0;
    l_lh: "width", "1lh" => 19.2;
    l_vw: "width", "10vw" => 128.0;
    l_vh: "width", "10vh" => 80.0;
    l_vmin: "width", "10vmin" => 80.0;
    l_vmax: "width", "10vmax" => 128.0;
    l_svw: "width", "50svw" => 640.0;
    l_dvh: "width", "100dvh" => 800.0;
    l_unit_case: "width", "10PX" => 10.0;
    l_zero_unitless: "margin-top", "0" => 0.0;
    l_negative_margin: "margin-top", "-5px" => -5.0;
    l_calc_mixed: "width", "calc(1in + 4px)" => 100.0;
    l_calc_em: "width", "calc(2em * 2)" => 64.0;
    l_min: "width", "min(10px, 2em)" => 10.0;
    l_max: "width", "max(10px, 2em)" => 32.0;
    l_clamp: "width", "clamp(5px, 1em, 12px)" => 12.0;
    l_nested_calc: "width", "calc(calc(1px + 1px) * (2 + 1))" => 6.0;
    l_round_nearest: "width", "round(nearest, 23px, 10px)" => 20.0;
    l_round_up: "width", "round(up, 23px, 10px)" => 30.0;
    l_round_down: "width", "round(down, 28px, 10px)" => 20.0;
    l_mod: "width", "mod(18px, 5px)" => 3.0;
    l_rem_fn: "width", "rem(18px, 5px)" => 3.0;
    l_abs: "margin-top", "abs(-12px)" => 12.0;
    l_hypot: "width", "hypot(3px, 4px)" => 5.0;
    l_trig_sin: "opacity", "sin(90deg)" => 1.0;
    l_trig_cos: "opacity", "cos(0deg)" => 1.0;
    l_pow_sqrt: "opacity", "calc(sqrt(16) / pow(2, 3))" => 0.5;
    l_sign: "opacity", "sign(-20px)" => -1.0;
}

macro_rules! invalid_value_cases {
    ($($name:ident: $prop:expr, $src:expr;)*) => {
        $(
            #[test]
            fn $name() {
                assert!(parse_value($prop, $src).is_err(), "{}: {:?} should fail", $prop, $src);
            }
        )*
    };
}

invalid_value_cases! {
    iv_unitless_nonzero: "width", "10";
    iv_negative_width: "width", "-1px";
    iv_negative_padding: "padding-top", "-1px";
    iv_unknown_unit: "width", "10zz";
    iv_angle_for_length: "width", "10deg";
    iv_display_unknown: "display", "sideways";
    iv_display_number: "display", "1";
    iv_display_repeat: "display", "block block";
    iv_display_four: "display", "block flow list-item inline";
    iv_opacity_word: "opacity", "half";
    iv_z_index_real: "z-index", "1.5";
    iv_calc_type_mismatch: "width", "calc(1px + 2)";
    iv_calc_divide_by_length: "width", "calc(1px / 1px)";
    iv_calc_no_space_plus: "width", "calc(1px+2px)";
    iv_calc_empty: "width", "calc()";
    iv_unknown_property: "frobnicate", "1";
    iv_empty: "width", "";
    iv_two_lengths_single: "width", "1px 2px";
    iv_margin_five: "margin", "1px 2px 3px 4px 5px";
    iv_bad_url: "background-image", "url(a b)";
}

macro_rules! valid_value_cases {
    ($($name:ident: $prop:expr, $src:expr;)*) => {
        $(
            #[test]
            fn $name() {
                if let Err(e) = parse_value($prop, $src) {
                    panic!("{}: {:?} should parse: {e:?}", $prop, $src);
                }
            }
        )*
    };
}

valid_value_cases! {
    vv_display_block: "display", "block";
    vv_display_case: "display", "FLEX";
    vv_position: "position", "sticky";
    vv_width_auto: "width", "auto";
    vv_width_percent: "width", "50%";
    vv_margin_four: "margin", "1px 2px 3px 4px";
    vv_margin_auto: "margin", "0 auto";
    vv_opacity: "opacity", "0.5";
    vv_opacity_percent: "opacity", "50%";
    vv_z_index: "z-index", "-3";
    vv_z_index_auto: "z-index", "auto";
    vv_line_height_number: "line-height", "1.5";
    vv_font_weight: "font-weight", "700";
    vv_transition_duration: "transition-duration", "1s, 200ms";
    vv_background_image_url: "background-image", "url(a.png)";
    vv_font_family: "font-family", "'Open Sans', Arial, sans-serif";
    vv_grid_template: "grid-template-columns", "[a] 1fr [b] minmax(10px, 2fr)";
    vv_transform: "transform", "translate(10px, 20%) rotate(45deg)";
    vv_box_shadow: "box-shadow", "0 0 4px rgba(0, 0, 0, .5), inset 1px 1px red";
    vv_content_string: "content", "'\\201C'";
    vv_aspect_ratio: "aspect-ratio", "16 / 9";
    vv_border_color: "border-top-color", "#abc";
    vv_color_keyword_inherit_style: "color", "inherit";
    vv_resolution_function: "background-image", "image-set('a.png' 1x, 'b.png' 2dppx)";
    vv_display_two_keywords: "display", "inline flex";
    vv_display_list_item: "display", "block flow list-item";
}

#[test]
fn unknown_property_name_errors() {
    assert!(parse_value("x-unused", "1").is_err());
}

#[test]
fn global_keywords() {
    for (src, want) in [
        ("initial", GlobalKeyword::Initial),
        ("INHERIT", GlobalKeyword::Inherit),
        ("unset", GlobalKeyword::Unset),
        ("revert", GlobalKeyword::Revert),
        ("revert-layer", GlobalKeyword::RevertLayer),
    ] {
        assert_eq!(
            parse_value("width", src).unwrap(),
            PropertyValue::Global(want),
            "{src}"
        );
    }
}

#[test]
fn global_keyword_with_other_tokens_is_invalid() {
    assert!(parse_value("width", "inherit 1px").is_err());
}

#[test]
fn custom_property_kept_raw() {
    let v = parse_value("--anything", " { weird ] stuff").unwrap();
    assert!(matches!(v, PropertyValue::Custom(_)));
}

#[test]
fn custom_property_global_keyword() {
    assert_eq!(
        parse_value("--x", "inherit").unwrap(),
        PropertyValue::Global(GlobalKeyword::Inherit)
    );
}

#[test]
fn var_is_left_unresolved() {
    assert!(matches!(
        parse_value("width", "var(--w, 10px)").unwrap(),
        PropertyValue::Unresolved(_)
    ));
}

#[test]
fn var_inside_calc_is_left_unresolved() {
    assert!(matches!(
        parse_value("width", "calc(var(--w) * 2)").unwrap(),
        PropertyValue::Unresolved(_)
    ));
}

#[test]
fn env_is_left_unresolved() {
    assert!(matches!(
        parse_value("padding-top", "env(safe-area-inset-top)").unwrap(),
        PropertyValue::Unresolved(_)
    ));
}

#[test]
fn url_value() {
    assert_eq!(
        one("background-image", "url(\"a b.png\")"),
        Value::Url("a b.png".into())
    );
    assert_eq!(
        one("background-image", "url(x.png)"),
        Value::Url("x.png".into())
    );
}

#[test]
fn string_value() {
    assert_eq!(one("content", "'hi'"), Value::String("hi".into()));
}

#[test]
fn length_value_shape() {
    assert_eq!(
        one("width", "2.5em"),
        Value::Length(Length {
            value: 2.5,
            unit: LengthUnit::Em
        })
    );
}

#[test]
fn percentage_resolves_against_basis() {
    let v = one("width", "25%");
    let ctx = ResolveContext {
        percent_basis: Some(200.0),
        ..ResolveContext::default()
    };
    assert_eq!(v.resolve(&ctx), Some(50.0));
    assert_eq!(v.resolve(&ResolveContext::default()), None);
}

#[test]
fn calc_percentage_resolves_against_basis() {
    let v = one("width", "calc(50% - 10px)");
    let ctx = ResolveContext {
        percent_basis: Some(100.0),
        ..ResolveContext::default()
    };
    assert_eq!(v.resolve(&ctx), Some(40.0));
}

#[test]
fn length_unit_names_round_trip() {
    for name in [
        "px", "em", "rem", "vw", "cqw", "lh", "rlh", "vb", "svmax", "Q",
    ] {
        let u = LengthUnit::from_name(name).unwrap_or_else(|| panic!("{name}"));
        assert!(u.name().eq_ignore_ascii_case(name));
    }
    assert!(LengthUnit::from_name("zz").is_none());
}

#[test]
fn container_units_use_container_size() {
    let ctx = ResolveContext {
        container_width: 500.0,
        container_height: 100.0,
        ..ResolveContext::default()
    };
    assert_eq!(LengthUnit::Cqw.to_px(10.0, &ctx), 50.0);
    assert_eq!(LengthUnit::Cqh.to_px(10.0, &ctx), 10.0);
}

// Math functions.

#[test]
fn calc_types() {
    assert_eq!(calc("calc(1 + 2)").ty, CalcType::Number);
    assert_eq!(calc("calc(1px + 2em)").ty, CalcType::Length);
    assert_eq!(calc("calc(10% + 2px)").ty, CalcType::LengthPercentage);
    assert_eq!(calc("calc(10%)").ty, CalcType::Percentage);
    assert_eq!(calc("calc(1deg + 1turn)").ty, CalcType::Angle);
    assert_eq!(calc("calc(1s - 10ms)").ty, CalcType::Time);
}

#[test]
fn calc_simplifies_constants() {
    let c = calc("calc((1 + 2) * 3 - 4 / 2)");
    assert!(c.as_leaf().is_some());
    assert_eq!(c.resolve(&ResolveContext::default()), 7.0);
}

#[test]
fn calc_unit_conversions_fold() {
    let c = calc("calc(1in - 6px)");
    assert_eq!(c.resolve(&ResolveContext::default()), 90.0);
}

#[test]
fn calc_angles_in_degrees() {
    assert_eq!(
        calc("calc(0.5turn + 10deg)").resolve(&ResolveContext::default()),
        190.0
    );
}

#[test]
fn calc_errors() {
    assert!(calc_err("calc(1px * 1px)"));
    assert!(calc_err("calc(1px + 1s)"));
    assert!(calc_err("calc(1 / 0px)"));
    assert!(calc_err("calc(1px -2px)"));
    assert!(calc_err("calc(* 2)"));
    assert!(calc_err("clamp(1px, 2px)"));
    assert!(calc_err("min()"));
}

#[test]
fn calc_division_by_zero_clamps_to_largest_finite() {
    let c = calc("calc(1px / 0)");
    assert_eq!(c.resolve(&ResolveContext::default()), f64::MAX);
    let c = calc("calc(-1px / 0)");
    assert_eq!(c.resolve(&ResolveContext::default()), -f64::MAX);
    let c = calc("calc(0px / 0)");
    assert_eq!(c.resolve(&ResolveContext::default()), 0.0);
}

#[test]
fn calc_depth_limited() {
    let mut src = String::new();
    for _ in 0..350 {
        src.push_str("calc(");
    }
    src.push_str("1px");
    for _ in 0..350 {
        src.push(')');
    }
    let v = parse_component_values(&src, &Options::default()).unwrap();
    let e = parse_math(&v[0], &Limits::default()).unwrap_err();
    assert_eq!(e.limit, Some(anvil_css::LimitKind::ValueDepth));
}

// Property table.

#[test]
fn property_table_size() {
    assert!(PROPERTIES.len() >= 400, "{}", PROPERTIES.len());
}

#[test]
fn property_table_sorted_and_unique() {
    for w in PROPERTIES.windows(2) {
        assert!(w[0].name < w[1].name, "{} then {}", w[0].name, w[1].name);
    }
}

#[test]
fn property_names_are_lowercase_idents() {
    for p in PROPERTIES {
        assert!(
            p.name.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
            "{}",
            p.name
        );
        assert!(!p.initial.is_empty(), "{}", p.name);
    }
}

#[test]
fn inherited_flags() {
    for name in [
        "color",
        "font-size",
        "font-family",
        "line-height",
        "visibility",
        "cursor",
        "text-align",
        "white-space",
        "direction",
        "letter-spacing",
    ] {
        assert!(is_inherited(name), "{name} should inherit");
    }
    for name in [
        "display",
        "width",
        "margin-top",
        "background-color",
        "position",
        "z-index",
        "opacity",
        "border-top-width",
        "overflow-x",
        "transform",
    ] {
        assert!(!is_inherited(name), "{name} should not inherit");
    }
}

#[test]
fn property_lookup_case_insensitive() {
    assert!(property("COLOR").is_some());
    assert!(property("nope").is_none());
}

#[test]
fn shorthands_marked() {
    for name in [
        "margin",
        "padding",
        "border",
        "background",
        "font",
        "flex",
        "grid",
        "transition",
        "animation",
        "inset",
    ] {
        assert!(property(name).is_some_and(|p| p.shorthand), "{name}");
    }
    assert!(!property("margin-top").unwrap().shorthand);
}

#[test]
fn every_property_initial_value_parses_or_is_shorthand() {
    let mut failures = Vec::new();
    for p in PROPERTIES {
        if p.shorthand {
            continue;
        }
        if parse_value(p.name, p.initial).is_err() {
            failures.push(format!("{}: {}", p.name, p.initial));
        }
    }
    assert!(
        failures.is_empty(),
        "initial values that do not parse: {failures:#?}"
    );
}
