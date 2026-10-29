// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Media query, container query, and @supports tests, written from
//! Media Queries Level 4/5, CSS Containment Level 3, and CSS Conditional
//! Rules Level 3/4.

use anvil_css::media::{
    ColorScheme, ContainerEnv, MediaEnv, MediaType, Pointer, Tri, parse_container_prelude,
    parse_media_query_list_str, parse_supports_condition,
};
use anvil_css::parser::parse_component_values;
use anvil_css::{Limits, Options};

fn eval(src: &str) -> bool {
    parse_media_query_list_str(src).evaluate(&MediaEnv::default())
}

fn eval_in(src: &str, env: &MediaEnv) -> bool {
    parse_media_query_list_str(src).evaluate(env)
}

macro_rules! media_cases {
    ($($name:ident: $src:expr => $want:expr;)*) => {
        $(
            #[test]
            fn $name() {
                assert_eq!(eval($src), $want, "query {:?} against the default environment", $src);
            }
        )*
    };
}

// The default environment: screen, 1280x800, 1dppx, 8-bit color, light,
// hover, fine pointer, scripting enabled.
media_cases! {
    q_empty_list_matches: "" => true;
    q_all: "all" => true;
    q_screen: "screen" => true;
    q_print: "print" => false;
    q_print_does_not_match_screen: "print and (min-width: 1px)" => false;
    q_unknown_type: "tv" => false;
    q_deprecated_type: "handheld" => false;
    q_type_case: "SCREEN" => true;
    q_not_print: "not print" => true;
    q_not_screen: "not screen" => false;
    q_only_screen: "only screen" => true;
    q_list_any: "print, screen" => true;
    q_min_width: "(min-width: 1000px)" => true;
    q_min_width_fail: "(min-width: 1300px)" => false;
    q_max_width: "(max-width: 1280px)" => true;
    q_width_exact: "(width: 1280px)" => true;
    q_width_em: "(min-width: 80em)" => true;
    q_width_em_fail: "(min-width: 81em)" => false;
    q_height: "(max-height: 800px)" => true;
    q_range_lt: "(width < 1281px)" => true;
    q_range_lt_fail: "(width < 1280px)" => false;
    q_range_le: "(width <= 1280px)" => true;
    q_range_gt: "(width > 1279px)" => true;
    q_range_ge: "(width >= 1280px)" => true;
    q_range_reversed: "(1000px < width)" => true;
    q_range_double: "(1000px < width <= 1280px)" => true;
    q_range_double_fail: "(1000px < width < 1280px)" => false;
    q_range_double_ge: "(1300px > width >= 1280px)" => true;
    q_range_eq: "(width = 1280px)" => true;
    q_and: "screen and (min-width: 100px) and (max-width: 2000px)" => true;
    q_and_fail: "screen and (min-width: 100px) and (max-width: 200px)" => false;
    q_or: "(max-width: 10px) or (min-width: 100px)" => true;
    q_not_condition: "not (max-width: 10px)" => true;
    q_nested_parens: "((min-width: 1px) and ((color) or (monochrome)))" => true;
    q_orientation: "(orientation: landscape)" => true;
    q_orientation_portrait: "(orientation: portrait)" => false;
    q_aspect_ratio: "(min-aspect-ratio: 16/10)" => true;
    q_aspect_ratio_exact: "(aspect-ratio: 8/5)" => true;
    q_aspect_ratio_fail: "(min-aspect-ratio: 2/1)" => false;
    q_resolution: "(min-resolution: 1dppx)" => true;
    q_resolution_dpi: "(min-resolution: 96dpi)" => true;
    q_resolution_fail: "(min-resolution: 2x)" => false;
    q_color_bool: "(color)" => true;
    q_color_bits: "(min-color: 8)" => true;
    q_monochrome_bool: "(monochrome)" => false;
    q_grid: "(grid)" => false;
    q_hover: "(hover: hover)" => true;
    q_hover_bool: "(hover)" => true;
    q_any_hover_none: "(any-hover: none)" => false;
    q_pointer_fine: "(pointer: fine)" => true;
    q_pointer_coarse: "(pointer: coarse)" => false;
    q_any_pointer: "(any-pointer: fine)" => true;
    q_scheme_light: "(prefers-color-scheme: light)" => true;
    q_scheme_dark: "(prefers-color-scheme: dark)" => false;
    q_reduced_motion: "(prefers-reduced-motion: reduce)" => false;
    q_reduced_motion_none: "(prefers-reduced-motion: no-preference)" => true;
    q_forced_colors: "(forced-colors: active)" => false;
    q_scripting: "(scripting: enabled)" => true;
    q_display_mode: "(display-mode: browser)" => true;
    q_update: "(update: fast)" => true;
    q_unknown_feature_false: "(x-unknown: 1)" => false;
    q_not_unknown_feature_still_false: "not (x-unknown: 1)" => false;
    q_unknown_or_true: "(x-unknown: 1) or (min-width: 1px)" => true;
    q_invalid_query_not_all: "screen and" => false;
    q_invalid_one_of_list: "screen and, print" => false;
    q_invalid_then_valid: "@@, screen" => true;
    q_mixing_and_or_invalid: "(color) and (hover) or (grid)" => false;
    q_bad_value: "(min-width: red)" => false;
    q_min_with_range_syntax_invalid: "(min-width > 10px)" => false;
    q_unitless_zero: "(min-width: 0)" => true;
    q_unitless_nonzero_invalid: "(min-width: 10)" => false;
    q_calc_value: "(min-width: calc(500px + 1em))" => true;
    q_whitespace_le_invalid: "(width < = 1280px)" => false;
}

#[test]
fn print_environment() {
    let env = MediaEnv {
        media_type: MediaType::Print,
        ..MediaEnv::default()
    };
    assert!(eval_in("print", &env));
    assert!(!eval_in("screen", &env));
    assert!(eval_in("all", &env));
}

#[test]
fn dark_environment() {
    let env = MediaEnv {
        color_scheme: ColorScheme::Dark,
        ..MediaEnv::default()
    };
    assert!(eval_in("(prefers-color-scheme: dark)", &env));
}

#[test]
fn narrow_environment() {
    let env = MediaEnv {
        width: 375.0,
        height: 667.0,
        ..MediaEnv::default()
    };
    assert!(eval_in("(max-width: 600px)", &env));
    assert!(eval_in("(orientation: portrait)", &env));
}

#[test]
fn coarse_pointer_environment() {
    let env = MediaEnv {
        pointer: Pointer::Coarse,
        hover: false,
        ..MediaEnv::default()
    };
    assert!(eval_in("(pointer: coarse)", &env));
    assert!(eval_in("(hover: none)", &env));
    assert!(!eval_in("(hover)", &env));
}

#[test]
fn em_uses_environment_font_size() {
    let env = MediaEnv {
        font_size: 20.0,
        ..MediaEnv::default()
    };
    assert!(eval_in("(min-width: 64em)", &env));
    assert!(!eval_in("(min-width: 65em)", &env));
}

#[test]
fn unknown_feature_is_unknown_not_false() {
    let l = parse_media_query_list_str("(x-unknown: 1)");
    assert_eq!(l.0[0].evaluate_tri(&MediaEnv::default()), Tri::Unknown);
}

#[test]
fn invalid_query_is_marked() {
    let l = parse_media_query_list_str("screen and, print");
    assert!(l.0[0].invalid);
    assert!(!l.0[1].invalid);
}

#[test]
fn deep_nesting_in_media_is_bounded() {
    let mut src = String::new();
    for _ in 0..400 {
        src.push('(');
    }
    src.push_str("color");
    for _ in 0..400 {
        src.push(')');
    }
    let l = parse_media_query_list_str(&src);
    assert!(l.0[0].invalid);
}

// Container queries.

fn container(src: &str) -> (Option<String>, Option<Tri>) {
    let v = parse_component_values(src, &Options::default()).unwrap();
    let (name, cond) = parse_container_prelude(&v, &Limits::default())
        .unwrap_or_else(|e| panic!("{src:?}: {e:?}"));
    let env = ContainerEnv {
        width: 500.0,
        height: 300.0,
        ..ContainerEnv::default()
    };
    (name, cond.map(|c| c.evaluate_container(&env)))
}

#[test]
fn container_width() {
    assert_eq!(container("(min-width: 400px)"), (None, Some(Tri::True)));
    assert_eq!(container("(width > 600px)"), (None, Some(Tri::False)));
}

#[test]
fn container_named_condition() {
    assert_eq!(
        container("card (inline-size >= 500px)"),
        (Some("card".into()), Some(Tri::True))
    );
}

#[test]
fn container_aspect_and_orientation() {
    assert_eq!(container("(orientation: landscape)").1, Some(Tri::True));
    assert_eq!(container("(aspect-ratio > 1)").1, Some(Tri::True));
}

#[test]
fn container_block_size() {
    assert_eq!(container("(block-size < 200px)").1, Some(Tri::False));
}

#[test]
fn container_and_or_not() {
    assert_eq!(
        container("(width > 1px) and (height > 1px)").1,
        Some(Tri::True)
    );
    assert_eq!(container("not (width > 1000px)").1, Some(Tri::True));
    assert_eq!(
        container("(width > 1000px) or (height < 1000px)").1,
        Some(Tri::True)
    );
}

#[test]
fn container_style_query_unknown() {
    assert_eq!(container("style(--x: 1)").1, Some(Tri::Unknown));
}

#[test]
fn container_bad_names() {
    for src in [
        "none (width > 1px)",
        "and (width > 1px)",
        "inherit",
        "(width > 1px) trailing",
    ] {
        let v = parse_component_values(src, &Options::default()).unwrap();
        assert!(
            parse_container_prelude(&v, &Limits::default()).is_err(),
            "{src:?}"
        );
    }
}

// @supports.

fn supports(src: &str) -> Option<bool> {
    let v = parse_component_values(src, &Options::default()).unwrap();
    parse_supports_condition(&v, &Limits::default())
        .ok()
        .map(|c| c.evaluate())
}

macro_rules! supports_cases {
    ($($name:ident: $src:expr => $want:expr;)*) => {
        $(
            #[test]
            fn $name() {
                assert_eq!(supports($src), $want, "condition {:?}", $src);
            }
        )*
    };
}

supports_cases! {
    sup_known_declaration: "(display: grid)" => Some(true);
    sup_bad_value: "(display: sideways)" => Some(false);
    sup_unknown_property: "(x-foo: 1)" => Some(false);
    sup_custom_property: "(--anything: whatever)" => Some(true);
    sup_important_allowed: "(color: red !important)" => Some(true);
    sup_not: "not (display: sideways)" => Some(true);
    sup_and: "(display: grid) and (color: red)" => Some(true);
    sup_and_fail: "(display: grid) and (color: nope)" => Some(false);
    sup_or: "(display: nope) or (color: red)" => Some(true);
    sup_nested: "((display: grid) or (display: nope)) and (not (color: nope))" => Some(true);
    sup_selector: "selector(a > b:has(c))" => Some(true);
    sup_selector_bad: "selector(a!!)" => Some(false);
    sup_selector_pseudo_element: "selector(::before)" => Some(true);
    sup_unknown_function: "x-thing(1)" => Some(false);
    sup_general_enclosed: "(x y z)" => Some(false);
    sup_var_value: "(width: var(--w))" => Some(true);
    sup_calc_value: "(width: calc(1px + 2em))" => Some(true);
    sup_mixed_and_or_invalid: "(a: b) and (c: d) or (e: f)" => None;
    sup_bare_declaration_invalid: "display: grid" => None;
    sup_empty_invalid: "" => None;
    sup_not_without_space_is_general_enclosed: "not(display: grid)" => Some(false);
    sup_important_bad_value: "(color: nope !important)" => Some(false);
}
