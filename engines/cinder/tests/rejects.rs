// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Source texts the parser must reject, one test per early error or syntax
//! error.

#![forbid(unsafe_code)]

mod common;
use common::{module_err, script_err};

macro_rules! bad_scripts {
    ($($name:ident: $src:expr;)*) => {
        $(#[test] fn $name() { script_err($src); })*
    };
}

macro_rules! bad_modules {
    ($($name:ident: $src:expr;)*) => {
        $(#[test] fn $name() { module_err($src); })*
    };
}

bad_scripts! {
    // plain syntax
    missing_paren: "if (a { }";
    unclosed_block: "{ a;";
    unclosed_paren_call: "f(a, b";
    stray_close: "a; }";
    two_exprs_one_line: "a b";
    bad_member: "a.;";
    bad_spread_in_paren: "(...a);";
    empty_parens: "();";
    trailing_comma_in_paren: "(a,);";
    unterminated_string: "'abc";
    unterminated_template: "`abc${x}";
    unterminated_regex: "x = /abc";
    template_bad_escape_untagged: "`\\unicode`";
    template_octal_untagged: "`\\01`";
    number_then_ident: "3in []";
    // declarations
    const_without_init: "const a;";
    destructure_without_init: "let [a];";
    let_let: "let let = 1;";
    duplicate_let: "let a; let a;";
    let_then_var: "let a; var a;";
    var_then_let: "var a; let a;";
    const_then_function: "const f = 1; function f() {}";
    class_then_let: "class A {} let A;";
    block_let_var_conflict: "{ let a; var a; }";
    block_var_into_let: "let a; { var a; }";
    param_let_conflict: "function f(a) { let a; }";
    catch_param_let_conflict: "try {} catch (e) { let e; }";
    catch_pattern_var_conflict: "try {} catch ([e]) { var e; }";
    catch_duplicate_params: "try {} catch ([a, a]) {}";
    switch_duplicate_let: "switch (x) { case 1: let a; case 2: let a; }";
    for_let_body_var: "for (let i of x) { var i; }";
    strict_block_function_dupes: "'use strict'; { function f() {} function f() {} }";
    block_generator_dupes: "{ function* f() {} function* f() {} }";
    function_decl_no_name: "function () {}";
    class_decl_no_name: "class {}";
    // assignment targets
    assign_to_literal: "1 = 2;";
    assign_to_call: "f() = 1;";
    assign_to_paren_object: "({a}) = 1;";
    compound_assign_to_pattern: "[a] += 1;";
    logical_assign_to_pattern: "({a} ??= b);";
    update_literal: "1++;";
    update_call: "++f();";
    prefix_update_paren_pattern: "++[a];";
    assign_to_optional_chain: "a?.b = 1;";
    update_optional_chain: "a?.b++;";
    for_in_bad_target: "for (f() in x) ;";
    for_of_let_target: "for (let.x of y) ;";
    for_async_of: "for (async of x) ;";
    for_of_initializer: "for (var a = 1 of b) ;";
    for_in_let_initializer: "for (let a = 1 in b) ;";
    for_of_two_bindings: "for (let a, b of c) ;";
    for_await_in_sync: "function f() { for await (x of y) ; }";
    for_await_in_script_top: "for await (x of y) ;";
    cover_init_outside_pattern: "({a = 1});";
    cover_init_in_call: "f({a = 1});";
    cover_init_member_target: "[{a = 1}.b] = c;";
    duplicate_proto: "({__proto__: 1, __proto__: 2});";
    rest_not_last_array: "[...a, b] = c;";
    rest_trailing_comma_array: "[...a,] = c;";
    rest_trailing_comma_object: "({...a,} = c);";
    rest_with_default: "[...a = 1] = b;";
    object_rest_pattern_target: "({...{a}} = b);";
    method_as_target: "({m() {}} = a);";
    // operators
    nullish_mixed_or: "a ?? b || c;";
    nullish_mixed_and: "a && b ?? c;";
    unary_before_exponent: "-a ** b;";
    typeof_before_exponent: "typeof a ** b;";
    arrow_as_operand: "a || () => 1;";
    arrow_call_same_line: "() => {}();";
    not_arrow: "!() => 1;";
    new_arrow: "new () => 1;";
    arrow_newline_before: "(a)\n=> a;";
    arrow_dup_params: "(a, a) => 1;";
    arrow_rest_not_last: "(...a, b) => 1;";
    arrow_rest_trailing_comma: "(...a,) => 1;";
    arrow_member_param: "(a.b) => 1;";
    arrow_paren_param: "((a)) => 1;";
    optional_chain_template: "a?.b`c`;";
    optional_chain_template_newline: "a?.b\n`c`";
    new_optional_chain: "new a?.b();";
    private_in_wrong_place: "class A { #x; m() { return 1 + #x in this; } }";
    private_without_in: "class A { #x; m() { #x; } }";
    // keywords and names
    reserved_as_var: "var if = 1;";
    reserved_as_param: "function f(class) {}";
    escaped_keyword_as_var: "var \\u0069f = 1;";
    escaped_keyword_statement: "\\u0069f (a) ;";
    enum_reserved: "var enum;";
    reserved_shorthand: "({if});";
    escaped_reserved_shorthand: "({\\u0069f});";
    // control flow
    break_outside: "break;";
    continue_outside: "continue;";
    continue_in_switch_only: "switch (a) { case 1: continue; }";
    break_unknown_label: "while (1) break nope;";
    continue_block_label: "l: { while (1) continue l; }";
    duplicate_label: "a: a: ;";
    label_across_function: "l: while (1) { (function () { break l; }); }";
    return_at_top: "return 1;";
    throw_newline: "throw\nnew Error();";
    try_alone: "try {}";
    two_defaults: "switch (a) { default: default: }";
    if_lexical_body: "if (a) let x = 1;";
    while_const_body: "while (a) const x = 1;";
    loop_function_body: "while (a) function f() {}";
    let_bracket_statement: "if (a) let [b] = c;";
    let_bracket_newline_redeclare: "var a; let\n[a] = [1]";
    labeled_generator: "l: function* g() {}";
    if_async_function: "if (a) async function f() {}";
    class_in_statement_position: "if (a) class A {}";
    // functions
    rest_default_param: "function f(...a = 1) {}";
    rest_not_last_param: "function f(...a, b) {}";
    rest_trailing_comma_param: "function f(...a,) {}";
    use_strict_non_simple: "function f(a = 1) { 'use strict'; }";
    use_strict_dup_params: "function f(a, a) { 'use strict'; }";
    use_strict_eval_param: "function f(eval) { 'use strict'; }";
    use_strict_function_name: "function arguments() { 'use strict'; }";
    dup_params_with_default: "function f(a, a = 1) {}";
    dup_params_method: "({ m(a, a) {} });";
    getter_with_param: "({ get x(a) {} });";
    setter_no_param: "({ set x() {} });";
    setter_two_params: "({ set x(a, b) {} });";
    setter_rest_param: "({ set x(...a) {} });";
    yield_in_generator_params: "function* g(a = yield) {}";
    await_in_async_params: "async function f(a = await 1) {}";
    await_in_async_arrow_params: "async (a = await 1) => a;";
    await_name_async_arrow_param: "async (await) => 1;";
    await_name_async_arrow_single: "async await => 1;";
    yield_name_in_generator: "function* g() { var yield; }";
    await_name_in_async: "async function f() { var await; }";
    yield_in_arrow_params_in_generator: "function* g() { (a = yield) => a; }";
    super_call_outside_ctor: "function f() { super(); }";
    super_call_in_base_ctor: "class A { constructor() { super(); } }";
    super_prop_in_function: "function f() { return super.x; }";
    super_prop_at_top: "super.x;";
    super_alone: "class A extends B { m() { super; } }";
    super_optional: "class A extends B { m() { super?.x; } }";
    new_target_at_top: "new.target;";
    new_target_in_arrow_top: "() => new.target;";
    new_bad_meta: "function f() { new.foo; }";
    import_meta_in_script: "import.meta;";
    new_import_call: "new import('x');";
    import_decl_in_script: "import a from 'a';";
    export_in_script: "export var a;";
    // classes
    class_dup_constructor: "class A { constructor() {} constructor() {} }";
    class_getter_constructor: "class A { get constructor() {} }";
    class_generator_constructor: "class A { *constructor() {} }";
    class_async_constructor: "class A { async constructor() {} }";
    class_field_constructor: "class A { constructor = 1; }";
    class_static_prototype_method: "class A { static prototype() {} }";
    class_static_prototype_field: "class A { static prototype = 1; }";
    class_private_constructor: "class A { #constructor() {} }";
    class_private_dupe: "class A { #a; #a; }";
    class_private_get_get: "class A { get #a() {} get #a() {} }";
    class_private_static_mismatch: "class A { get #a() {} static set #a(v) {} }";
    class_undeclared_private: "class A { m() { return this.#nope; } }";
    private_outside_class: "this.#x;";
    delete_private: "class A { #x; m() { delete this.#x; } }";
    class_arguments_in_field: "class A { x = arguments; }";
    class_arguments_in_static_block: "class A { static { arguments; } }";
    class_await_in_static_block: "class A { static { var await; } }";
    class_return_in_static_block: "class A { static { return; } }";
    class_strict_with: "class A { m() { with (a) ; } }";
    class_name_yield: "class yield {}";
    class_name_let: "class let {}";
    class_extends_arrow: "class A extends () => {} {}";
    class_method_dup_params: "class A { m(a, a) {} }";
    class_field_asi_fail: "class A { a b }";
    // strict mode
    strict_with: "'use strict'; with (a) ;";
    strict_octal_number: "'use strict'; 017;";
    strict_leading_zero_decimal: "'use strict'; 08;";
    strict_octal_escape: "'use strict'; '\\07';";
    strict_eight_escape: "'use strict'; '\\8';";
    octal_directive_before_use_strict: "function f() { '\\07'; 'use strict'; }";
    strict_delete_ident: "'use strict'; delete x;";
    strict_delete_paren_ident: "'use strict'; delete (x);";
    strict_assign_eval: "'use strict'; eval = 1;";
    strict_assign_arguments: "'use strict'; arguments++;";
    strict_var_eval: "'use strict'; var eval;";
    strict_reserved_let: "'use strict'; var let;";
    strict_reserved_static: "'use strict'; var static;";
    strict_reserved_implements: "'use strict'; implements = 1;";
    strict_yield_name: "'use strict'; var yield;";
    strict_dup_params: "'use strict'; function f(a, a) {}";
    strict_labeled_function: "'use strict'; l: function f() {}";
    strict_if_function: "'use strict'; if (a) function f() {}";
    strict_catch_eval: "'use strict'; try {} catch (eval) {}";
    strict_destructure_arguments: "'use strict'; [arguments] = [];";
    strict_arrow_eval_param: "'use strict'; (eval) => 1;";
    // numbers, regex
    regex_bad_flag: "/a/x;";
    regex_dup_flag: "/a/gg;";
    regex_u_and_v: "/a/uv;";
    bigint_fraction: "1.5n;";
    numeric_separator_double: "1__0;";
    numeric_separator_trailing: "1_;";
    // misc
    yield_star_newline_ok_but_bad_operand: "function* g() { yield\n* 1; }";
    async_newline_arrow: "async\n(a) => a;";
    getter_private_in_object: "({ get #x() {} });";
    shorthand_number: "({1});";
    object_missing_colon: "({'a'});";
    unexpected_eof_after_operator: "a +";
    deep_unclosed: "((((";
}

bad_modules! {
    module_await_name: "var await;";
    module_await_ref_in_function: "function f() { await; }";
    module_with: "with (a) ;";
    module_octal: "017;";
    module_html_close_comment: "x\n--> y";
    module_dup_export: "export var a; export var a;";
    module_dup_export_names: "var a, b; export {a as x, b as x};";
    module_dup_default: "export default 1; export default 2;";
    module_export_undeclared: "export {nope};";
    module_export_string_local: "export {'a'};";
    module_export_reserved_local: "export {if};";
    module_import_reserved_binding: "import {if} from 'x';";
    module_import_string_without_as: "import {'a'} from 'x';";
    module_import_dup_binding: "import {a, b as a} from 'x';";
    module_import_then_let: "import a from 'x'; let a;";
    module_import_in_block: "{ import a from 'x'; }";
    module_export_in_function: "function f() { export var a; }";
    module_dup_attribute: "import a from 'x' with { type: 'json', type: 'json' };";
    module_import_missing_from: "import {a} 'x';";
    module_lone_surrogate_export_name: "var a; export {a as '\\uD800'};";
    module_top_function_let_conflict: "function f() {} let f;";
    module_top_function_dupes: "function f() {} function f() {}";
    module_await_in_static_block: "class A { static { await 1; } }";
    module_export_default_let_decl: "export default let x = 1;";
    module_export_expression_without_default: "export 1;";
}

#[test]
fn error_positions_point_at_the_problem() {
    let e = script_err("var x = 1;\nvar y = ;");
    assert_eq!((e.span.line, e.span.column), (2, 9));
    let e = script_err("let a;\n  let a;");
    assert_eq!((e.span.line, e.span.column), (2, 7));
    let e = script_err("x = `\n\\u{zz}`");
    assert_eq!(e.span.line, 2);
}

#[test]
fn error_display_is_line_colon_column() {
    let e = script_err("a b");
    assert_eq!(e.to_string(), "1:3: expected `;` but found `b`");
}

#[test]
fn error_messages_are_plain() {
    assert!(script_err("break;").message.contains("inside a loop"));
    assert!(script_err("const a;").message.contains("initializer"));
    assert!(
        script_err("let a; let a;")
            .message
            .contains("already been declared")
    );
    assert!(script_err("a ?? b || c").message.contains("??"));
}
