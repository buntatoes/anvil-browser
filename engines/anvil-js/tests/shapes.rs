// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Checks on the shape of the tree: precedence, ASI, regex versus division,
//! templates, patterns, and spans.

#![forbid(unsafe_code)]

mod common;
use anvil_js::ast::*;
use common::{module, script};

fn only_expr(src: &str) -> Expr {
    let p = script(src);
    assert_eq!(p.body.len(), 1, "{src}: {:?}", p.body);
    match p.body.into_iter().next().map(|mut s| s.take_kind()) {
        Some(StmtKind::Expr { expr, .. }) => *expr,
        other => panic!("{src}: {other:?}"),
    }
}

/// A compact prefix form of an expression, enough to check structure.
fn sexp(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Ident(n) => n.clone(),
        ExprKind::Num(n) => format!("{n}"),
        ExprKind::Str(s) => format!("{:?}", String::from_utf16_lossy(s)),
        ExprKind::Binary { op, left, right } => format!("({op:?} {} {})", sexp(left), sexp(right)),
        ExprKind::Logical { op, left, right } => format!("({op:?} {} {})", sexp(left), sexp(right)),
        ExprKind::Unary { op, arg } => format!("({op:?} {})", sexp(arg)),
        ExprKind::Update { op, prefix, arg } => format!(
            "({op:?}{} {})",
            if *prefix { "Pre" } else { "Post" },
            sexp(arg)
        ),
        ExprKind::Assign { op, value, .. } => format!("({op:?} _ {})", sexp(value)),
        ExprKind::Cond { test, cons, alt } => {
            format!("(? {} {} {})", sexp(test), sexp(cons), sexp(alt))
        }
        ExprKind::Paren(inner) => format!("[{}]", sexp(inner)),
        ExprKind::Seq(items) => format!(
            "(, {})",
            items.iter().map(sexp).collect::<Vec<_>>().join(" ")
        ),
        ExprKind::Call {
            callee,
            args,
            optional,
        } => format!(
            "(call{} {} {})",
            if *optional { "?" } else { "" },
            sexp(callee),
            args.iter().map(sexp).collect::<Vec<_>>().join(" ")
        ),
        ExprKind::Member {
            object,
            prop,
            optional,
        } => {
            let p = match prop {
                MemberProp::Name(i) => i.name.clone(),
                MemberProp::Computed(c) => format!("[{}]", sexp(c)),
                MemberProp::Private(i) => format!("#{}", i.name),
            };
            format!(
                "(.{} {} {p})",
                if *optional { "?" } else { "" },
                sexp(object)
            )
        }
        ExprKind::OptionalChain(inner) => format!("(chain {})", sexp(inner)),
        ExprKind::New { callee, args } => format!("(new {} {})", sexp(callee), args.len()),
        ExprKind::Regex { pattern, flags } => format!("/{pattern}/{flags}"),
        ExprKind::Await(a) => format!("(await {})", sexp(a)),
        ExprKind::Spread(a) => format!("...{}", sexp(a)),
        other => format!("{other:?}").chars().take(12).collect(),
    }
}

fn shape(src: &str) -> String {
    sexp(&only_expr(src))
}

#[test]
fn precedence_mul_over_add() {
    assert_eq!(shape("a + b * c"), "(Add a (Mul b c))");
}

#[test]
fn left_associative_subtraction() {
    assert_eq!(shape("a - b - c"), "(Sub (Sub a b) c)");
}

#[test]
fn exponent_is_right_associative() {
    assert_eq!(shape("a ** b ** c"), "(Exp a (Exp b c))");
}

#[test]
fn exponent_binds_tighter_than_unary_on_right() {
    assert_eq!(shape("a ** -b"), "(Exp a (Minus b))");
}

#[test]
fn logical_precedence() {
    assert_eq!(shape("a || b && c"), "(Or a (And b c))");
    assert_eq!(shape("a ?? b ?? c"), "(Nullish (Nullish a b) c)");
}

#[test]
fn bitwise_precedence() {
    assert_eq!(shape("a | b ^ c & d"), "(BitOr a (BitXor b (BitAnd c d)))");
}

#[test]
fn equality_below_relational() {
    assert_eq!(shape("a == b < c"), "(Eq a (Lt b c))");
}

#[test]
fn shift_between_relational_and_additive() {
    assert_eq!(shape("a < b << c + d"), "(Lt a (Shl b (Add c d)))");
}

#[test]
fn conditional_is_right_nested() {
    assert_eq!(shape("a ? b : c ? d : e"), "(? a b (? c d e))");
}

#[test]
fn assignment_is_right_associative() {
    assert_eq!(shape("a = b = c"), "(Assign _ (Assign _ c))");
}

#[test]
fn comma_is_lowest() {
    assert_eq!(shape("a = 1, b"), "(, (Assign _ 1) b)");
}

#[test]
fn in_operator_parsed() {
    assert_eq!(shape("'x' in o"), "(In \"x\" o)");
}

#[test]
fn unary_chain() {
    assert_eq!(shape("!-~a"), "(Not (Minus (BitNot a)))");
}

#[test]
fn typeof_typeof() {
    assert_eq!(shape("typeof typeof a"), "(Typeof (Typeof a))");
}

#[test]
fn postfix_and_prefix_update() {
    assert_eq!(shape("a++"), "(IncPost a)");
    assert_eq!(shape("--a"), "(DecPre a)");
}

#[test]
fn member_call_chain() {
    assert_eq!(shape("a.b(c)[d]"), "(. (call (. a b) c) [d])");
}

#[test]
fn optional_chain_wraps_whole_chain() {
    assert_eq!(shape("a?.b.c"), "(chain (. (.? a b) c))");
    assert_eq!(shape("a?.(b)"), "(chain (call? a b))");
}

#[test]
fn new_with_and_without_args() {
    assert_eq!(shape("new A"), "(new A 0)");
    assert_eq!(shape("new A(1, 2)"), "(new A 2)");
    assert_eq!(shape("new A.b(1)"), "(new (. A b) 1)");
    assert_eq!(shape("new A()()"), "(call (new A 0) )");
    assert_eq!(shape("new new A()()"), "(new (new A 0) 0)");
}

#[test]
fn await_in_async_function() {
    let p = script("async function f() { await a + b; }");
    let StmtKind::Function(f) = &p.body[0].kind else {
        panic!()
    };
    let StmtKind::Expr { expr, .. } = &f.body[0].kind else {
        panic!()
    };
    assert_eq!(sexp(expr), "(Add (await a) b)");
}

// ---- regex versus division ----

#[test]
fn division_after_identifier() {
    assert_eq!(shape("a / b / c"), "(Div (Div a b) c)");
}

#[test]
fn regex_at_expression_start() {
    assert_eq!(shape("/b/g"), "/b/g");
}

#[test]
fn regex_after_operator() {
    assert_eq!(shape("a = /=/"), "(Assign _ /=/)");
}

#[test]
fn division_assign_after_identifier() {
    assert_eq!(shape("a /= 2"), "(Div _ 2)");
}

#[test]
fn regex_starting_with_equals() {
    assert_eq!(shape("x(/=a/)"), "(call x /=a/)");
}

#[test]
fn division_after_close_paren() {
    assert_eq!(shape("(a) / 2"), "(Div [a] 2)");
}

#[test]
fn division_after_close_bracket() {
    assert_eq!(shape("a[0] / 2 / 1"), "(Div (Div (. a [0]) 2) 1)");
}

#[test]
fn division_after_postfix() {
    assert_eq!(shape("a++ / 2"), "(Div (IncPost a) 2)");
}

#[test]
fn regex_after_typeof() {
    assert_eq!(shape("typeof /x/"), "(Typeof /x/)");
}

#[test]
fn regex_after_block_statement() {
    let p = script("{}\n/foo/g");
    assert_eq!(p.body.len(), 2);
    let StmtKind::Expr { expr, .. } = &p.body[1].kind else {
        panic!()
    };
    assert_eq!(sexp(expr), "/foo/g");
}

#[test]
fn regex_with_slash_in_class() {
    assert_eq!(shape("/[/]+/"), "/[/]+/");
}

#[test]
fn regex_after_return_keyword() {
    let p = script("function f() { return /x/.test(y); }");
    let StmtKind::Function(f) = &p.body[0].kind else {
        panic!()
    };
    assert!(matches!(f.body[0].kind, StmtKind::Return(Some(_))));
}

#[test]
fn division_after_number() {
    assert_eq!(shape("4 / 2"), "(Div 4 2)");
}

// ---- ASI ----

fn count(src: &str) -> usize {
    script(src).body.len()
}

#[test]
fn asi_newline_splits_statements() {
    assert_eq!(count("a\nb"), 2);
}

#[test]
fn asi_not_before_open_paren() {
    assert_eq!(count("a\n(b)"), 1);
}

#[test]
fn asi_not_before_template() {
    assert_eq!(count("a\n`b`"), 1);
}

#[test]
fn asi_not_before_bracket() {
    assert_eq!(count("a\n[b]"), 1);
}

#[test]
fn asi_before_prefix_increment() {
    let p = script("a\n++b");
    assert_eq!(p.body.len(), 2);
}

#[test]
fn asi_after_return() {
    let p = script("function f() { return\na }");
    let StmtKind::Function(f) = &p.body[0].kind else {
        panic!()
    };
    assert!(matches!(f.body[0].kind, StmtKind::Return(None)));
    assert_eq!(f.body.len(), 2);
}

#[test]
fn asi_after_break_with_label_on_next_line() {
    let p = script("x: while (1) { break\nx }");
    let StmtKind::Labeled { body, .. } = &p.body[0].kind else {
        panic!()
    };
    let StmtKind::While { body, .. } = &body.kind else {
        panic!()
    };
    let StmtKind::Block(stmts) = &body.kind else {
        panic!()
    };
    assert!(matches!(stmts[0].kind, StmtKind::Break(None)));
}

#[test]
fn asi_at_end_of_input() {
    assert_eq!(count("a = 1"), 1);
}

#[test]
fn asi_before_close_brace() {
    assert_eq!(count("{ a } b"), 2);
}

#[test]
fn asi_not_in_for_header() {
    common::script_err("for (a\nb\n) ;");
}

#[test]
fn asi_does_not_make_empty_statement() {
    common::script_err("if (a)\nelse b");
}

#[test]
fn asi_after_do_while() {
    assert_eq!(count("do x; while (y) z"), 2);
}

#[test]
fn asi_arrow_block_then_call_on_next_line() {
    assert_eq!(count("var f = () => {}\n(1)"), 2);
}

#[test]
fn asi_class_field_lines() {
    let p = script("class A { a = 1\n b\n c() {} }");
    let StmtKind::Class(c) = &p.body[0].kind else {
        panic!()
    };
    assert_eq!(c.body.len(), 3);
}

// ---- templates ----

fn template_of(src: &str) -> Template {
    match only_expr(src).take_kind() {
        ExprKind::Template(t) => t,
        ExprKind::TaggedTemplate { quasi, .. } => quasi,
        other => panic!("{other:?}"),
    }
}

#[test]
fn template_parts_and_exprs() {
    let t = template_of("`a${b}c${d}e`");
    assert_eq!(t.quasis.len(), 3);
    assert_eq!(t.exprs.len(), 2);
    assert_eq!(t.quasis[1].raw, "c");
}

#[test]
fn template_nested_three_deep() {
    let t = template_of("`1${`2${`3${x}`}`}`");
    let ExprKind::Template(inner) = &t.exprs[0].kind else {
        panic!()
    };
    let ExprKind::Template(inner2) = &inner.exprs[0].kind else {
        panic!()
    };
    assert_eq!(inner2.quasis[0].raw, "3");
}

#[test]
fn template_object_literal_inside_sub() {
    let t = template_of("`${ {a: `}`}.a }`");
    assert_eq!(t.exprs.len(), 1);
}

#[test]
fn template_block_arrow_inside_sub() {
    let t = template_of("`${ (() => { return 1 })() }x`");
    assert_eq!(t.quasis[1].raw, "x");
}

#[test]
fn template_empty() {
    let t = template_of("``");
    assert_eq!(t.quasis.len(), 1);
    assert_eq!(t.quasis[0].cooked.as_deref(), Some(&[][..]));
}

#[test]
fn tagged_template_keeps_bad_escape_raw() {
    let t = template_of("tag`\\u{zz}`");
    assert!(t.quasis[0].cooked.is_none());
    assert_eq!(t.quasis[0].raw, "\\u{zz}");
}

#[test]
fn template_cooked_crlf_normalised() {
    let t = template_of("`a\r\nb`");
    assert_eq!(
        String::from_utf16_lossy(t.quasis[0].cooked.as_ref().unwrap()),
        "a\nb"
    );
}

#[test]
fn template_regex_inside_sub() {
    let t = template_of("`${/}/.source}`");
    assert!(matches!(t.exprs[0].kind, ExprKind::Member { .. }));
}

// ---- unicode ----

#[test]
fn unicode_escape_identifier_decoded() {
    assert_eq!(shape("\\u0061bc"), "abc");
    assert_eq!(shape("\\u{1d4b6}"), "\u{1d4b6}");
}

#[test]
fn non_ascii_identifier() {
    assert_eq!(shape("naïve"), "naïve");
}

#[test]
fn string_keeps_lone_surrogate() {
    let e = only_expr("'\\uDC00'");
    assert_eq!(e.kind, ExprKind::Str(vec![0xDC00]));
}

#[test]
fn escaped_contextual_keyword_is_identifier() {
    // `l\u0065t` is the name `let`, not the keyword, in sloppy mode.
    let p = script("var l\\u0065t = 1;");
    assert_eq!(p.body.len(), 1);
}

#[test]
fn zero_width_joiner_in_identifier() {
    assert_eq!(shape("a\u{200D}b"), "a\u{200D}b");
}

// ---- patterns and other trees ----

#[test]
fn destructuring_assignment_becomes_pattern() {
    let e = only_expr("[a, {b: c = 1}, ...d] = e");
    let ExprKind::Assign { target, .. } = &e.kind else {
        panic!()
    };
    let PatKind::Array(items) = &target.kind else {
        panic!()
    };
    assert_eq!(items.len(), 3);
    assert!(matches!(items[2].as_ref().unwrap().kind, PatKind::Rest(_)));
    let names: Vec<String> = target.bound_names().into_iter().map(|i| i.name).collect();
    assert_eq!(names, ["a", "c", "d"]);
}

#[test]
fn shorthand_default_becomes_shorthand_pattern() {
    let e = only_expr("({a = 1} = b)");
    let ExprKind::Paren(inner) = &e.kind else {
        panic!()
    };
    let ExprKind::Assign { target, .. } = &inner.kind else {
        panic!()
    };
    let PatKind::Object(props) = &target.kind else {
        panic!()
    };
    assert!(matches!(
        props[0],
        PatProp::Prop {
            shorthand: true,
            ..
        }
    ));
}

#[test]
fn arrow_params_from_cover() {
    let e = only_expr("({a}, [b], c = 1, ...d) => a");
    let ExprKind::Arrow(f) = &e.kind else {
        panic!()
    };
    assert_eq!(f.params.len(), 4);
    assert!(matches!(f.params[3].kind, PatKind::Rest(_)));
    assert!(matches!(f.body, ArrowBody::Expr(_)));
}

#[test]
fn async_arrow_flags() {
    let e = only_expr("async (a) => await a");
    let ExprKind::Arrow(f) = &e.kind else {
        panic!()
    };
    assert!(f.is_async);
}

#[test]
fn async_call_is_call() {
    assert_eq!(shape("async(a, ...b)"), "(call async a ...b)");
}

#[test]
fn function_flags() {
    let p = script("async function* g(a, b = 1) { yield await a; }");
    let StmtKind::Function(f) = &p.body[0].kind else {
        panic!()
    };
    assert!(f.is_async && f.is_generator);
    assert_eq!(f.params.len(), 2);
    assert_eq!(f.id.as_ref().map(|i| i.name.as_str()), Some("g"));
}

#[test]
fn function_strict_and_simple_flags() {
    let p = script("function f(a) { 'use strict'; } function g({a}) {}");
    let StmtKind::Function(f) = &p.body[0].kind else {
        panic!()
    };
    assert!(f.strict && f.simple_params);
    let StmtKind::Function(g) = &p.body[1].kind else {
        panic!()
    };
    assert!(!g.strict && !g.simple_params);
}

#[test]
fn directive_recorded() {
    let p = script("'use strict'; 'other'; x");
    assert!(p.strict);
    assert!(
        matches!(&p.body[0].kind, StmtKind::Expr { directive: Some(d), .. } if d == "use strict")
    );
    assert!(matches!(&p.body[1].kind, StmtKind::Expr { directive: Some(d), .. } if d == "other"));
    assert!(matches!(
        &p.body[2].kind,
        StmtKind::Expr {
            directive: None,
            ..
        }
    ));
}

#[test]
fn escaped_use_strict_is_not_a_directive_switch() {
    let p = script("'use\\x20strict'; with (a) ;");
    assert!(!p.strict);
}

#[test]
fn parenthesized_string_is_not_directive() {
    let p = script("('use strict'); with (a) ;");
    assert!(!p.strict);
}

#[test]
fn class_members_kinds() {
    let p = script(
        "class A extends B { constructor() { super(); } static #x = 1; get y() {} static { } }",
    );
    let StmtKind::Class(c) = &p.body[0].kind else {
        panic!()
    };
    assert!(c.super_class.is_some());
    assert!(matches!(
        c.body[0].kind,
        ClassMemberKind::Method {
            kind: MethodKind::Constructor,
            ..
        }
    ));
    assert!(matches!(
        c.body[1].kind,
        ClassMemberKind::Field {
            is_static: true,
            key: PropKey::Private(_),
            ..
        }
    ));
    assert!(matches!(
        c.body[2].kind,
        ClassMemberKind::Method {
            kind: MethodKind::Get,
            ..
        }
    ));
    assert!(matches!(c.body[3].kind, ClassMemberKind::StaticBlock(_)));
}

#[test]
fn for_of_await_flag() {
    let p = module("for await (const x of y) ;");
    assert!(matches!(
        p.body[0].kind,
        StmtKind::ForOf { is_await: true, .. }
    ));
}

#[test]
fn import_specifier_kinds() {
    let p = module("import a, {b as c, 'd e' as f} from 'm';");
    let StmtKind::Import(imp) = &p.body[0].kind else {
        panic!()
    };
    assert_eq!(imp.specifiers.len(), 3);
    assert!(matches!(imp.specifiers[0], ImportSpec::Default(_)));
    assert_eq!(String::from_utf16_lossy(&imp.source.value), "m");
}

#[test]
fn export_default_kinds() {
    let p = module("export default class {}");
    assert!(matches!(
        &p.body[0].kind,
        StmtKind::Export(ExportDecl::Default(ExportDefault::Class(_)))
    ));
    let p = module("export default (1)");
    assert!(matches!(
        &p.body[0].kind,
        StmtKind::Export(ExportDecl::Default(ExportDefault::Expr(_)))
    ));
}

#[test]
fn module_is_strict() {
    assert!(module("x").strict);
    assert_eq!(module("x").source_type, SourceType::Module);
}

#[test]
fn spans_cover_statements() {
    let src = "let x = 1;\nfoo(x);";
    let p = script(src);
    assert_eq!(&src[p.body[0].span.start..p.body[0].span.end], "let x = 1;");
    assert_eq!(&src[p.body[1].span.start..p.body[1].span.end], "foo(x);");
    assert_eq!(p.body[1].span.line, 2);
}

#[test]
fn spans_cover_binary() {
    let src = "  a +  b";
    let e = only_expr(src);
    assert_eq!(&src[e.span.start..e.span.end], "a +  b");
    assert_eq!(e.span.column, 3);
}

#[test]
fn bigint_literal_value() {
    let e = only_expr("0x1Fn");
    assert_eq!(
        e.kind,
        ExprKind::BigInt(BigIntLit {
            radix: 16,
            digits: "1f".into()
        })
    );
}

#[test]
fn numeric_value_with_separators() {
    assert_eq!(only_expr("1_000.5e1").kind, ExprKind::Num(10005.0));
}

#[test]
fn labeled_statement_tree() {
    let p = script("a: b: for (;;) break a;");
    let StmtKind::Labeled { label, body } = &p.body[0].kind else {
        panic!()
    };
    assert_eq!(label.name, "a");
    assert!(matches!(body.kind, StmtKind::Labeled { .. }));
}

#[test]
fn private_in_tree() {
    let p = script("class A { #x; m(o) { return #x in o; } }");
    let StmtKind::Class(c) = &p.body[0].kind else {
        panic!()
    };
    let ClassMemberKind::Method { value, .. } = &c.body[1].kind else {
        panic!()
    };
    let StmtKind::Return(Some(e)) = &value.body[0].kind else {
        panic!()
    };
    assert!(matches!(e.kind, ExprKind::PrivateIn { .. }));
}
