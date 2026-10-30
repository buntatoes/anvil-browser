// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Syntax trees built by hand, dumped without spans. Each tree is the one
// the source in tests/parse/trees/ of the same name stands for, and
// ast_dump.test.expected is what the reference prints for those sources
// (check.sh compares the two), so the dump format and the node set are
// checked against the reference without the parser in between.

import {
  A_ASSIGN,
  MK_GET,
  VK_CONST,
  VK_VAR,
  makeArrow,
  makeArrowBodyExpr,
  makeClass,
  makeExportDefault,
  makeExportDefaultExpr,
  makeExprArrow,
  makeExprAssign,
  makeExprAwait,
  makeExprCall,
  makeExprIdent,
  makeExprMember,
  makeExprNum,
  makeExprOptChain,
  makeExprParen,
  makeExprSpread,
  makeExprStr,
  makeExprTagged,
  makeExprThis,
  makeForHeadVar,
  makeFunction,
  makeIdent,
  makeImportDecl,
  makeImportSpecDefault,
  makeImportSpecNamed,
  makeKeyIdent,
  makeKeyPrivate,
  makeMemberField,
  makeMemberMethod,
  makeMemberStaticBlock,
  makeModuleNameIdent,
  makePatArray,
  makePatAssign,
  makePatIdent,
  makePatObject,
  makePatPropProp,
  makePatRest,
  makeProgram,
  makePropName,
  makePropPrivate,
  makeStmtClass,
  makeStmtContinue,
  makeStmtExport,
  makeStmtExpr,
  makeStmtForOf,
  makeStmtIf,
  makeStmtImport,
  makeStmtLabeled,
  makeStmtReturn,
  makeStmtVar,
  makeStrLit,
  makeTemplate,
  makeTemplateElement,
  makeVarDecl,
  makeVarDeclarator,
} from "../../src/syntax/ast.js";
import { dumpProgram } from "../../src/syntax/ast_dump.js";
import { unitsOf } from "../../src/syntax/token_dump.js";

// Spans do not show in these dumps; every node gets 0, 0, 1, 1.

function u(text) {
  return unitsOf(text);
}

function id(name) {
  return makeIdent(u(name), 0, 0, 1, 1);
}

function ref(name) {
  return makeExprIdent(u(name), 0, 0, 1, 1);
}

function pat(name) {
  return makePatIdent(u(name), 0, 0, 1, 1);
}

function num(v) {
  return makeExprNum(v, 0, 0, 1, 1);
}

function stmt(e) {
  return makeStmtExpr(e, null, 0, 0, 1, 1);
}

function show(name, prog) {
  console.log(`-- ${name}`);
  dumpProgram(prog, false, null, (l) => console.log(l));
}

function script(body, strict) {
  return makeProgram(false, strict, body, 0, 0, 1, 1);
}

show("t01_empty.js", script([], false));

show("t02_expr.js", script([stmt(ref("x"))], false));

show(
  "t03_var.js",
  script(
    [
      makeStmtVar(
        makeVarDecl(
          VK_VAR,
          [makeVarDeclarator(pat("a"), num(1), 0, 0, 1, 1), makeVarDeclarator(pat("b"), null, 0, 0, 1, 1)],
          0, 0, 1, 1,
        ),
        0, 0, 1, 1,
      ),
    ],
    false,
  ),
);

show("t04_if.js", script([makeStmtIf(ref("a"), stmt(ref("b")), stmt(ref("c")), 0, 0, 1, 1)], false));

const call = makeExprCall(ref("f"), [num(1), makeExprSpread(ref("a"), 0, 0, 1, 1)], false, 0, 0, 1, 1);
show(
  "t05_call.js",
  script(
    [stmt(makeExprOptChain(makeExprMember(call, makePropName(id("b"), 0, 0, 1, 1), true, 0, 0, 1, 1), 0, 0, 1, 1))],
    false,
  ),
);

const getter = makeFunction(
  null,
  [],
  [makeStmtReturn(makeExprMember(makeExprThis(0, 0, 1, 1), makePropPrivate(id("x"), 0, 0, 1, 1), false, 0, 0, 1, 1), 0, 0, 1, 1)],
  false,
  false,
  true,
  true,
  0, 0, 1, 1,
);
const cls = makeClass(
  id("A"),
  ref("B"),
  [
    makeMemberField(makeKeyPrivate(id("x"), 0, 0, 1, 1), false, num(1), 0, 0, 1, 1),
    makeMemberStaticBlock([], 0, 0, 1, 1),
    makeMemberMethod(makeKeyIdent(id("y"), 0, 0, 1, 1), MK_GET, false, getter, 0, 0, 1, 1),
  ],
  0, 0, 1, 1,
);
show("t06_class.js", script([makeStmtClass(cls, 0, 0, 1, 1)], false));

const imp = makeImportDecl(
  [
    makeImportSpecDefault(id("a"), 0, 0, 1, 1),
    makeImportSpecNamed(makeModuleNameIdent(id("b"), 0, 0, 1, 1), id("c"), 0, 0, 1, 1),
  ],
  makeStrLit(u("m"), 0, 0, 1, 1),
  [],
  0, 0, 1, 1,
);
const exp = makeExportDefault(makeExportDefaultExpr(num(1), 0, 0, 1, 1), 0, 0, 1, 1);
show(
  "t07_module.mjs",
  makeProgram(true, true, [makeStmtImport(imp, 0, 0, 1, 1), makeStmtExport(exp, 0, 0, 1, 1)], 0, 0, 1, 1),
);

const target = makePatObject(
  [
    makePatPropProp(makeKeyIdent(id("a"), 0, 0, 1, 1), pat("a"), true, 0, 0, 1, 1),
    makePatPropProp(
      makeKeyIdent(id("b"), 0, 0, 1, 1),
      makePatArray([makePatAssign(pat("c"), num(1), 0, 0, 1, 1), makePatRest(pat("d"), 0, 0, 1, 1)], 0, 0, 1, 1),
      false,
      0, 0, 1, 1,
    ),
  ],
  0, 0, 1, 1,
);
show("t08_pattern.js", script([stmt(makeExprParen(makeExprAssign(A_ASSIGN, target, ref("e"), 0, 0, 1, 1), 0, 0, 1, 1))], false));

const tpl = makeTemplate(
  [makeTemplateElement(u("a"), u("a"), 0, 0, 1, 1), makeTemplateElement(null, u("\\unicode"), 0, 0, 1, 1)],
  [ref("x")],
  0, 0, 1, 1,
);
show("t09_template.js", script([stmt(makeExprTagged(ref("tag"), tpl, 0, 0, 1, 1))], false));

const arrow = makeArrow([pat("a")], makeArrowBodyExpr(makeExprAwait(ref("a"), 0, 0, 1, 1), 0, 0, 1, 1), true, false, 0, 0, 1, 1);
show("t10_arrow.js", script([stmt(makeExprArrow(arrow, 0, 0, 1, 1))], false));

const loop = makeStmtForOf(
  makeForHeadVar(makeVarDecl(VK_CONST, [makeVarDeclarator(pat("k"), null, 0, 0, 1, 1)], 0, 0, 1, 1), 0, 0, 1, 1),
  ref("o"),
  makeStmtContinue(id("outer"), 0, 0, 1, 1),
  false,
  0, 0, 1, 1,
);
show(
  "t11_directive.js",
  script(
    [makeStmtExpr(makeExprStr(u("use strict"), 0, 0, 1, 1), u("use strict"), 0, 0, 1, 1), makeStmtLabeled(id("outer"), loop, 0, 0, 1, 1)],
    true,
  ),
);
