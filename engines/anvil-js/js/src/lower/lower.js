// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// AST lowering pass: transforms full ES2022+ syntax trees (destructuring,
// default/rest parameters, spread literals, optional chaining, logical
// assignments, computed/spread object properties, class fields/statics,
// for-in loops, parameterless/destructuring catch, and block functions)
// into Core JS syntax trees executable on AnvilVM.

import {
  S_EXPR,
  S_VAR,
  S_FUNCTION,
  S_CLASS,
  S_BLOCK,
  S_IF,
  S_FOR,
  S_FOR_IN,
  S_FOR_OF,
  S_WHILE,
  S_DO_WHILE,
  S_RETURN,
  S_SWITCH,
  S_LABELED,
  S_THROW,
  S_TRY,
  S_IMPORT,
  S_EXPORT,
  FI_VAR,
  FI_EXPR,
  FH_VAR,
  AB_EXPR,
  AB_BLOCK,
  CM_METHOD,
  CM_FIELD,
  CM_STATIC_BLOCK,
  PK_IDENT,
  PK_STR,
  PK_NUM,
  PK_COMPUTED,
  PK_PRIVATE,
  E_IDENT,
  E_THIS,
  E_SUPER,
  E_NULL,
  E_NUM,
  E_STR,
  E_TEMPLATE,
  E_TAGGED,
  E_ARRAY,
  E_OBJECT,
  E_FUNCTION,
  E_ARROW,
  E_CLASS,
  E_UNARY,
  E_UPDATE,
  E_BINARY,
  E_LOGICAL,
  E_ASSIGN,
  E_COND,
  E_CALL,
  E_NEW,
  E_MEMBER,
  E_OPT_CHAIN,
  E_SEQ,
  E_SPREAD,
  E_YIELD,
  E_AWAIT,
  E_PAREN,
  E_PRIVATE_IN,
  MP_NAME,
  MP_COMPUTED,
  MP_PRIVATE,
  PR_INIT,
  PR_SHORTHAND,
  PR_METHOD,
  PR_SPREAD,
  PT_IDENT,
  PT_ARRAY,
  PT_OBJECT,
  PT_ASSIGN,
  PT_REST,
  PT_EXPR,
  PP_PROP,
  PP_REST,
  ED_DECL,
  ED_DEFAULT,
  EX_FUNCTION,
  EX_CLASS,
  MK_METHOD,
  MK_CONSTRUCTOR,
  B_STRICT_EQ,
  B_STRICT_NOT_EQ,
  B_IN,
  L_AND,
  L_OR,
  A_ASSIGN,
  A_AND,
  A_OR,
  A_NULLISH,
  VK_LET,
  VK_CONST,
  makeProgram,
  makeIdent,
  makeStmtExpr,
  makeStmtVar,
  makeStmtFunction,
  makeStmtClass,
  makeStmtBlock,
  makeStmtEmpty,
  makeStmtIf,
  makeStmtFor,
  makeStmtForOf,
  makeStmtWhile,
  makeStmtDoWhile,
  makeStmtReturn,
  makeStmtSwitch,
  makeStmtLabeled,
  makeStmtThrow,
  makeStmtTry,
  makeVarDecl,
  makeVarDeclarator,
  makeForInitVar,
  makeForInitExpr,
  makeForHeadVar,
  makeSwitchCase,
  makeCatch,
  makeFunction,
  makeArrow,
  makeArrowBodyExpr,
  makeArrowBodyBlock,
  makeClass,
  makeMemberMethod,
  makeKeyIdent,
  makeExprIdent,
  makeExprThis,
  makeExprNull,
  makeExprNum,
  makeExprStr,
  makeExprArray,
  makeExprObject,
  makeExprFunction,
  makeExprArrow,
  makeExprUnary,
  makeExprUpdate,
  makeExprBinary,
  makeExprLogical,
  makeExprAssign,
  makeExprCond,
  makeExprCall,
  makeExprNew,
  makeExprMember,
  makeExprSeq,
  makeExprParen,
  makePropName,
  makePropComputed,
  makeObjPropInit,
  makePatIdent,
  makePatExpr,
} from "../syntax/ast.js";
import { unitsOf } from "../syntax/token_dump.js";
import { isIdStart, isIdContinue } from "../core/text.js";

const UNDEF_UNITS = unitsOf("undefined");
const PUSH_UNITS = unitsOf("push");
const CONSTRUCTOR_UNITS = unitsOf("constructor");
const THIS_CAPTURE_UNITS = unitsOf("__this");
const ANVIL_SLICE_UNITS = unitsOf("__anvil_slice");
const ANVIL_ASSIGN_UNITS = unitsOf("__anvil_assign");
const ANVIL_KEYS_UNITS = unitsOf("__anvil_keys");

function newState() {
  return {
    tempCounter: 0,
    blockDepth: 0,
    inArrow: false,
    arrowUsedThis: false,
  };
}

function freshUnits(st, prefix) {
  const id = st.tempCounter;
  st.tempCounter = id + 1;
  return unitsOf(`__${prefix}${id}`);
}

function concatArrays(a, b) {
  const out = [];
  for (const x of a) {
    out.push(x);
  }
  for (const y of b) {
    out.push(y);
  }
  return out;
}

function undefExpr() {
  return makeExprIdent(UNDEF_UNITS, 0, 0, 1, 1);
}

function identExpr(units) {
  return makeExprIdent(units, 0, 0, 1, 1);
}

function patIdent(units) {
  return makePatIdent(units, 0, 0, 1, 1);
}

function identNode(units) {
  return makeIdent(units, 0, 0, 1, 1);
}

function wrapIife(stmts) {
  const arrow = makeArrow([], makeArrowBodyBlock(stmts, 0, 0, 1, 1), false, true, 0, 0, 1, 1);
  const arrowExpr = makeExprArrow(arrow, 0, 0, 1, 1);
  return makeExprCall(arrowExpr, [], false, 0, 0, 1, 1);
}

function wrapArrowOne(paramUnits, bodyExpr, argExpr) {
  const arrow = makeArrow(
    [patIdent(paramUnits)],
    makeArrowBodyExpr(bodyExpr, 0, 0, 1, 1),
    false,
    true,
    0,
    0,
    1,
    1,
  );
  const arrowExpr = makeExprArrow(arrow, 0, 0, 1, 1);
  return makeExprCall(arrowExpr, [argExpr], false, 0, 0, 1, 1);
}

function isValidIdentUnits(units) {
  if (units.length === 0) {
    return false;
  }
  if (!isIdStart(units[0])) {
    return false;
  }
  let i = 1;
  while (i < units.length) {
    if (!isIdContinue(units[i])) {
      return false;
    }
    i += 1;
  }
  return true;
}

function isNullishExpr(valExpr) {
  const isNull = makeExprBinary(B_STRICT_EQ, valExpr, makeExprNull(0, 0, 1, 1), 0, 0, 1, 1);
  const isUndef = makeExprBinary(B_STRICT_EQ, valExpr, undefExpr(), 0, 0, 1, 1);
  return makeExprLogical(L_OR, isNull, isUndef, 0, 0, 1, 1);
}

function isNotNullishExpr(valExpr) {
  const notNull = makeExprBinary(B_STRICT_NOT_EQ, valExpr, makeExprNull(0, 0, 1, 1), 0, 0, 1, 1);
  const notUndef = makeExprBinary(B_STRICT_NOT_EQ, valExpr, undefExpr(), 0, 0, 1, 1);
  return makeExprLogical(L_AND, notNull, notUndef, 0, 0, 1, 1);
}

function memberByKey(st, objExpr, key) {
  const k = key.kind;
  if (k === PK_IDENT) {
    return makeExprMember(objExpr, makePropName(key.ident, 0, 0, 1, 1), false, 0, 0, 1, 1);
  }
  if (k === PK_PRIVATE) {
    const privUnits = concatArrays(unitsOf("__priv_"), key.ident.name);
    return makeExprMember(
      objExpr,
      makePropName(identNode(privUnits), 0, 0, 1, 1),
      false,
      0,
      0,
      1,
      1,
    );
  }
  if (k === PK_STR) {
    if (isValidIdentUnits(key.value)) {
      return makeExprMember(
        objExpr,
        makePropName(identNode(key.value), 0, 0, 1, 1),
        false,
        0,
        0,
        1,
        1,
      );
    }
    return makeExprMember(
      objExpr,
      makePropComputed(makeExprStr(key.value, 0, 0, 1, 1), 0, 0, 1, 1),
      false,
      0,
      0,
      1,
      1,
    );
  }
  if (k === PK_NUM) {
    return makeExprMember(
      objExpr,
      makePropComputed(makeExprNum(key.value, 0, 0, 1, 1), 0, 0, 1, 1),
      false,
      0,
      0,
      1,
      1,
    );
  }
  if (k === PK_COMPUTED) {
    return makeExprMember(
      objExpr,
      makePropComputed(lowerExpr(st, key.expr), 0, 0, 1, 1),
      false,
      0,
      0,
      1,
      1,
    );
  }
  return objExpr;
}

function makeSimpleVarStmt(varKind, nameUnits, initExpr) {
  const decl = makeVarDeclarator(patIdent(nameUnits), initExpr, 0, 0, 1, 1);
  const vd = makeVarDecl(varKind, [decl], 0, 0, 1, 1);
  return makeStmtVar(vd, 0, 0, 1, 1);
}

function lowerBindingPattern(st, pat, rhsExpr, varKind, outStmts) {
  const k = pat.kind;
  if (k === PT_IDENT) {
    const decl = makeVarDeclarator(pat, rhsExpr, pat.start, pat.end, pat.line, pat.col);
    const vd = makeVarDecl(varKind, [decl], pat.start, pat.end, pat.line, pat.col);
    outStmts.push(makeStmtVar(vd, pat.start, pat.end, pat.line, pat.col));
    return;
  }
  if (k === PT_ASSIGN) {
    const tmpUnits = freshUnits(st, "d");
    outStmts.push(makeSimpleVarStmt(VK_CONST, tmpUnits, rhsExpr));
    const tmpIdent = identExpr(tmpUnits);
    const cond = makeExprBinary(B_STRICT_EQ, tmpIdent, undefExpr(), 0, 0, 1, 1);
    const valExpr = makeExprCond(cond, lowerExpr(st, pat.def), tmpIdent, 0, 0, 1, 1);
    lowerBindingPattern(st, pat.target, valExpr, varKind, outStmts);
    return;
  }
  if (k === PT_ARRAY) {
    const arrUnits = freshUnits(st, "arr");
    outStmts.push(makeSimpleVarStmt(VK_CONST, arrUnits, rhsExpr));
    const arrIdent = identExpr(arrUnits);
    let i = 0;
    while (i < pat.items.length) {
      const item = pat.items[i];
      if (item !== null) {
        if (item.kind === PT_REST) {
          const sliceCall = makeExprCall(
            identExpr(ANVIL_SLICE_UNITS),
            [arrIdent, makeExprNum(i, 0, 0, 1, 1)],
            false,
            0,
            0,
            1,
            1,
          );
          lowerBindingPattern(st, item.arg, sliceCall, varKind, outStmts);
        } else {
          const elemExpr = makeExprMember(
            arrIdent,
            makePropComputed(makeExprNum(i, 0, 0, 1, 1), 0, 0, 1, 1),
            false,
            0,
            0,
            1,
            1,
          );
          lowerBindingPattern(st, item, elemExpr, varKind, outStmts);
        }
      }
      i += 1;
    }
    return;
  }
  if (k === PT_OBJECT) {
    const objUnits = freshUnits(st, "obj");
    outStmts.push(makeSimpleVarStmt(VK_CONST, objUnits, rhsExpr));
    const objIdent = identExpr(objUnits);
    for (const pp of pat.props) {
      if (pp.kind === PP_PROP) {
        const propVal = memberByKey(st, objIdent, pp.key);
        lowerBindingPattern(st, pp.value, propVal, varKind, outStmts);
      } else if (pp.kind === PP_REST) {
        lowerBindingPattern(st, pp.arg, objIdent, varKind, outStmts);
      }
    }
    return;
  }
  if (k === PT_REST) {
    lowerBindingPattern(st, pat.arg, rhsExpr, varKind, outStmts);
  }
}

function lowerAssignPattern(st, pat, rhsExpr, outStmts) {
  const k = pat.kind;
  if (k === PT_IDENT || k === PT_EXPR) {
    const asg = makeExprAssign(A_ASSIGN, pat, rhsExpr, 0, 0, 1, 1);
    outStmts.push(makeStmtExpr(asg, null, 0, 0, 1, 1));
    return;
  }
  if (k === PT_ASSIGN) {
    const tmpUnits = freshUnits(st, "da");
    outStmts.push(makeSimpleVarStmt(VK_CONST, tmpUnits, rhsExpr));
    const tmpIdent = identExpr(tmpUnits);
    const cond = makeExprBinary(B_STRICT_EQ, tmpIdent, undefExpr(), 0, 0, 1, 1);
    const valExpr = makeExprCond(cond, lowerExpr(st, pat.def), tmpIdent, 0, 0, 1, 1);
    lowerAssignPattern(st, pat.target, valExpr, outStmts);
    return;
  }
  if (k === PT_ARRAY) {
    const arrUnits = freshUnits(st, "da");
    outStmts.push(makeSimpleVarStmt(VK_CONST, arrUnits, rhsExpr));
    const arrIdent = identExpr(arrUnits);
    let i = 0;
    while (i < pat.items.length) {
      const item = pat.items[i];
      if (item !== null) {
        if (item.kind === PT_REST) {
          const sliceCall = makeExprCall(
            identExpr(ANVIL_SLICE_UNITS),
            [arrIdent, makeExprNum(i, 0, 0, 1, 1)],
            false,
            0,
            0,
            1,
            1,
          );
          lowerAssignPattern(st, item.arg, sliceCall, outStmts);
        } else {
          const elemExpr = makeExprMember(
            arrIdent,
            makePropComputed(makeExprNum(i, 0, 0, 1, 1), 0, 0, 1, 1),
            false,
            0,
            0,
            1,
            1,
          );
          lowerAssignPattern(st, item, elemExpr, outStmts);
        }
      }
      i += 1;
    }
    return;
  }
  if (k === PT_OBJECT) {
    const objUnits = freshUnits(st, "do");
    outStmts.push(makeSimpleVarStmt(VK_CONST, objUnits, rhsExpr));
    const objIdent = identExpr(objUnits);
    for (const pp of pat.props) {
      if (pp.kind === PP_PROP) {
        const propVal = memberByKey(st, objIdent, pp.key);
        lowerAssignPattern(st, pp.value, propVal, outStmts);
      } else if (pp.kind === PP_REST) {
        lowerAssignPattern(st, pp.arg, objIdent, outStmts);
      }
    }
  }
}

function lowerParamsAndPrologue(st, params) {
  const newParams = [];
  const prologue = [];
  let i = 0;
  while (i < params.length) {
    const p = params[i];
    if (p.kind === PT_IDENT) {
      newParams.push(p);
    } else if (p.kind === PT_ASSIGN && p.target.kind === PT_IDENT) {
      newParams.push(p.target);
      const pIdent = identExpr(p.target.name);
      const isUndef = makeExprBinary(B_STRICT_EQ, pIdent, undefExpr(), 0, 0, 1, 1);
      const asg = makeExprAssign(A_ASSIGN, p.target, lowerExpr(st, p.def), 0, 0, 1, 1);
      const ifStmt = makeStmtIf(
        isUndef,
        makeStmtBlock([makeStmtExpr(asg, null, 0, 0, 1, 1)], 0, 0, 1, 1),
        null,
        0,
        0,
        1,
        1,
      );
      prologue.push(ifStmt);
    } else if (p.kind === PT_REST) {
      const pUnits = freshUnits(st, "rest");
      newParams.push(patIdent(pUnits));
      const pIdent = identExpr(pUnits);
      const isUndef = makeExprBinary(B_STRICT_EQ, pIdent, undefExpr(), 0, 0, 1, 1);
      const arrVal = makeExprCond(
        isUndef,
        makeExprArray([], 0, 0, 1, 1),
        makeExprArray([pIdent], 0, 0, 1, 1),
        0,
        0,
        1,
        1,
      );
      lowerBindingPattern(st, p.arg, arrVal, VK_LET, prologue);
    } else {
      const pUnits = freshUnits(st, "p");
      newParams.push(patIdent(pUnits));
      lowerBindingPattern(st, p, identExpr(pUnits), VK_LET, prologue);
    }
    i += 1;
  }
  return { params: newParams, prologue: prologue };
}

function lowerFunctionNode(st, f) {
  const savedDepth = st.blockDepth;
  const savedInArrow = st.inArrow;
  const savedUsedThis = st.arrowUsedThis;
  st.blockDepth = 0;
  st.inArrow = false;
  st.arrowUsedThis = false;
  const pp = lowerParamsAndPrologue(st, f.params);
  let bodyStmts = concatArrays(pp.prologue, lowerStmtList(st, f.body));
  if (st.arrowUsedThis) {
    const thisDecl = makeSimpleVarStmt(VK_CONST, THIS_CAPTURE_UNITS, makeExprThis(0, 0, 1, 1));
    bodyStmts = injectInstanceFields(bodyStmts, [thisDecl], true);
  }
  st.blockDepth = savedDepth;
  st.inArrow = savedInArrow;
  st.arrowUsedThis = savedUsedThis;
  return makeFunction(
    f.id,
    pp.params,
    bodyStmts,
    false,
    false,
    f.strict,
    true,
    f.start,
    f.end,
    f.line,
    f.col,
  );
}

function lowerArrowNode(st, a) {
  const savedDepth = st.blockDepth;
  const savedInArrow = st.inArrow;
  st.blockDepth = 0;
  st.inArrow = true;
  const pp = lowerParamsAndPrologue(st, a.params);
  let body = null;
  if (a.body.kind === AB_EXPR && pp.prologue.length === 0) {
    body = makeArrowBodyExpr(lowerExpr(st, a.body.expr), a.body.start, a.body.end, a.body.line, a.body.col);
  } else if (a.body.kind === AB_EXPR) {
    const ret = makeStmtReturn(lowerExpr(st, a.body.expr), a.body.start, a.body.end, a.body.line, a.body.col);
    body = makeArrowBodyBlock(concatArrays(pp.prologue, [ret]), a.body.start, a.body.end, a.body.line, a.body.col);
  } else {
    const loweredBody = lowerStmtList(st, a.body.body);
    body = makeArrowBodyBlock(concatArrays(pp.prologue, loweredBody), a.body.start, a.body.end, a.body.line, a.body.col);
  }
  st.blockDepth = savedDepth;
  st.inArrow = savedInArrow;
  return makeArrow(pp.params, body, false, a.strict, a.start, a.end, a.line, a.col);
}

function injectInstanceFields(ctorBody, fieldStmts, hasSuper) {
  if (fieldStmts.length === 0) {
    return ctorBody;
  }
  if (!hasSuper) {
    return concatArrays(fieldStmts, ctorBody);
  }
  // In a derived constructor, place instance field initializers immediately
  // after the first top-level `super(...)` expression statement.
  const out = [];
  let inserted = false;
  for (const s of ctorBody) {
    out.push(s);
    if (
      !inserted &&
      s.kind === S_EXPR &&
      s.expr.kind === E_CALL &&
      s.expr.callee.kind === E_SUPER
    ) {
      for (const fs of fieldStmts) {
        out.push(fs);
      }
      inserted = true;
    }
  }
  if (!inserted) {
    return concatArrays(fieldStmts, ctorBody);
  }
  return out;
}

function lowerClassParts(st, c, classIdentUnits) {
  const methods = [];
  const instanceFieldStmts = [];
  const staticInitStmts = [];
  let ctorMethod = null;

  for (const m of c.body) {
    if (m.kind === CM_METHOD) {
      let key = m.key;
      if (key.kind === PK_PRIVATE) {
        const privUnits = concatArrays(unitsOf("__priv_"), key.ident.name);
        key = makeKeyIdent(identNode(privUnits), key.start, key.end, key.line, key.col);
      } else if (key.kind === PK_STR && isValidIdentUnits(key.value)) {
        key = makeKeyIdent(identNode(key.value), key.start, key.end, key.line, key.col);
      }
      const lf = lowerFunctionNode(st, m.value);
      if (m.isStatic) {
        const target = memberByKey(st, identExpr(classIdentUnits), key);
        const fnExpr = makeExprFunction(lf, m.start, m.end, m.line, m.col);
        const asg = makeExprAssign(A_ASSIGN, makePatExpr(target, 0, 0, 1, 1), fnExpr, 0, 0, 1, 1);
        staticInitStmts.push(makeStmtExpr(asg, null, 0, 0, 1, 1));
      } else if (m.methodKind === MK_CONSTRUCTOR) {
        ctorMethod = makeMemberMethod(key, MK_CONSTRUCTOR, false, lf, m.start, m.end, m.line, m.col);
        methods.push(ctorMethod);
      } else {
        methods.push(makeMemberMethod(key, MK_METHOD, false, lf, m.start, m.end, m.line, m.col));
      }
    } else if (m.kind === CM_FIELD) {
      const valExpr = m.value !== null ? lowerExpr(st, m.value) : undefExpr();
      if (m.isStatic) {
        const target = memberByKey(st, identExpr(classIdentUnits), m.key);
        const asg = makeExprAssign(A_ASSIGN, makePatExpr(target, 0, 0, 1, 1), valExpr, 0, 0, 1, 1);
        staticInitStmts.push(makeStmtExpr(asg, null, 0, 0, 1, 1));
      } else {
        const target = memberByKey(st, makeExprThis(0, 0, 1, 1), m.key);
        const asg = makeExprAssign(A_ASSIGN, makePatExpr(target, 0, 0, 1, 1), valExpr, 0, 0, 1, 1);
        instanceFieldStmts.push(makeStmtExpr(asg, null, 0, 0, 1, 1));
      }
    } else if (m.kind === CM_STATIC_BLOCK) {
      const blockBody = lowerStmtList(st, m.body);
      staticInitStmts.push(makeStmtBlock(blockBody, m.start, m.end, m.line, m.col));
    }
  }

  if (instanceFieldStmts.length > 0) {
    if (ctorMethod !== null) {
      if (ctorMethod.value.body.length > 0) {
        ctorMethod.value.body = injectInstanceFields(
          ctorMethod.value.body,
          instanceFieldStmts,
          c.superClass !== null,
        );
      } else {
        ctorMethod.value.body = instanceFieldStmts;
      }
    } else {
      const ctorBody = [];
      let params = [];
      if (c.superClass !== null) {
        const a0 = unitsOf("__a0");
        params = [patIdent(a0)];
        const supCall = makeExprCall(
          makeExprIdent(unitsOf("super"), 0, 0, 1, 1),
          [identExpr(a0)],
          false,
          0,
          0,
          1,
          1,
        );
        ctorBody.push(makeStmtExpr(supCall, null, 0, 0, 1, 1));
      }
      for (const fs of instanceFieldStmts) {
        ctorBody.push(fs);
      }
      const fn = makeFunction(null, params, ctorBody, false, false, true, true, 0, 0, 1, 1);
      const key = makeKeyIdent(identNode(CONSTRUCTOR_UNITS), 0, 0, 1, 1);
      methods.push(makeMemberMethod(key, MK_CONSTRUCTOR, false, fn, 0, 0, 1, 1));
    }
  }

  const superExpr = c.superClass !== null ? lowerExpr(st, c.superClass) : null;
  const clsNode = makeClass(
    identNode(classIdentUnits),
    superExpr,
    methods,
    c.start,
    c.end,
    c.line,
    c.col,
  );
  return { cls: clsNode, staticStmts: staticInitStmts };
}

function objectNeedsLowering(props) {
  for (const p of props) {
    if (p.kind === PR_SPREAD) {
      return true;
    }
    const key = p.kind === PR_SHORTHAND ? null : p.key;
    if (key !== null) {
      if (key.kind === PK_COMPUTED || key.kind === PK_NUM || key.kind === PK_PRIVATE) {
        return true;
      }
      if (key.kind === PK_STR && !isValidIdentUnits(key.value)) {
        return true;
      }
    }
  }
  return false;
}

function lowerObjectExpr(st, e) {
  if (!objectNeedsLowering(e.props)) {
    const newProps = [];
    const keyExprs = [];
    for (const p of e.props) {
      if (p.kind === PR_SHORTHAND) {
        const key = makeKeyIdent(p.ident, p.start, p.end, p.line, p.col);
        const val = makeExprIdent(p.ident.name, p.start, p.end, p.line, p.col);
        newProps.push(makeObjPropInit(key, val, p.start, p.end, p.line, p.col));
        keyExprs.push(makeExprStr(p.ident.name, 0, 0, 1, 1));
      } else if (p.kind === PR_METHOD) {
        const key =
          p.key.kind === PK_STR
            ? makeKeyIdent(identNode(p.key.value), p.key.start, p.key.end, p.key.line, p.key.col)
            : p.key;
        const fnExpr = makeExprFunction(lowerFunctionNode(st, p.value), p.start, p.end, p.line, p.col);
        newProps.push(makeObjPropInit(key, fnExpr, p.start, p.end, p.line, p.col));
      } else {
        const key =
          p.key.kind === PK_STR
            ? makeKeyIdent(identNode(p.key.value), p.key.start, p.key.end, p.key.line, p.key.col)
            : p.key;
        newProps.push(makeObjPropInit(key, lowerExpr(st, p.value), p.start, p.end, p.line, p.col));
        if (p.key.kind === PK_IDENT) {
          keyExprs.push(makeExprStr(p.key.ident.name, 0, 0, 1, 1));
        } else if (p.key.kind === PK_STR) {
          keyExprs.push(makeExprStr(p.key.value, 0, 0, 1, 1));
        }
      }
    }
    if (keyExprs.length > 0) {
      const keysIdent = makeKeyIdent(identNode(unitsOf("_keys")), 0, 0, 1, 1);
      newProps.push(makeObjPropInit(keysIdent, makeExprArray(keyExprs, 0, 0, 1, 1), 0, 0, 1, 1));
    }
    return makeExprObject(newProps, e.start, e.end, e.line, e.col);
  }

  const objUnits = freshUnits(st, "obj");
  const objIdent = identExpr(objUnits);
  const stmts = [makeSimpleVarStmt(VK_CONST, objUnits, makeExprObject([], 0, 0, 1, 1))];
  for (const p of e.props) {
    if (p.kind === PR_SPREAD) {
      const assignCall = makeExprCall(
        identExpr(ANVIL_ASSIGN_UNITS),
        [objIdent, lowerExpr(st, p.arg)],
        false,
        0,
        0,
        1,
        1,
      );
      stmts.push(makeStmtExpr(assignCall, null, 0, 0, 1, 1));
    } else if (p.kind === PR_SHORTHAND) {
      const target = makeExprMember(objIdent, makePropName(p.ident, 0, 0, 1, 1), false, 0, 0, 1, 1);
      const val = makeExprIdent(p.ident.name, p.start, p.end, p.line, p.col);
      const asg = makeExprAssign(A_ASSIGN, makePatExpr(target, 0, 0, 1, 1), val, 0, 0, 1, 1);
      stmts.push(makeStmtExpr(asg, null, 0, 0, 1, 1));
    } else if (p.kind === PR_METHOD) {
      const target = memberByKey(st, objIdent, p.key);
      const fnExpr = makeExprFunction(lowerFunctionNode(st, p.value), p.start, p.end, p.line, p.col);
      const asg = makeExprAssign(A_ASSIGN, makePatExpr(target, 0, 0, 1, 1), fnExpr, 0, 0, 1, 1);
      stmts.push(makeStmtExpr(asg, null, 0, 0, 1, 1));
    } else {
      const target = memberByKey(st, objIdent, p.key);
      const val = lowerExpr(st, p.value);
      const asg = makeExprAssign(A_ASSIGN, makePatExpr(target, 0, 0, 1, 1), val, 0, 0, 1, 1);
      stmts.push(makeStmtExpr(asg, null, 0, 0, 1, 1));
    }
  }
  stmts.push(makeStmtReturn(objIdent, 0, 0, 1, 1));
  return wrapIife(stmts);
}

function arrayHasSpread(items) {
  for (const x of items) {
    if (x !== null && x.kind === E_SPREAD) {
      return true;
    }
  }
  return false;
}

function lowerArrayExpr(st, e) {
  if (!arrayHasSpread(e.items)) {
    const newItems = [];
    for (const x of e.items) {
      if (x === null) {
        newItems.push(undefExpr());
      } else {
        newItems.push(lowerExpr(st, x));
      }
    }
    return makeExprArray(newItems, e.start, e.end, e.line, e.col);
  }
  const arrUnits = freshUnits(st, "a");
  const arrIdent = identExpr(arrUnits);
  const stmts = [makeSimpleVarStmt(VK_CONST, arrUnits, makeExprArray([], 0, 0, 1, 1))];
  for (const x of e.items) {
    if (x === null) {
      const pushMem = makeExprMember(arrIdent, makePropName(identNode(PUSH_UNITS), 0, 0, 1, 1), false, 0, 0, 1, 1);
      const call = makeExprCall(pushMem, [undefExpr()], false, 0, 0, 1, 1);
      stmts.push(makeStmtExpr(call, null, 0, 0, 1, 1));
    } else if (x.kind === E_SPREAD) {
      const elUnits = freshUnits(st, "el");
      const elIdent = identExpr(elUnits);
      const pushMem = makeExprMember(arrIdent, makePropName(identNode(PUSH_UNITS), 0, 0, 1, 1), false, 0, 0, 1, 1);
      const call = makeExprCall(pushMem, [elIdent], false, 0, 0, 1, 1);
      const loopHead = makeForHeadVar(
        makeVarDecl(VK_CONST, [makeVarDeclarator(patIdent(elUnits), null, 0, 0, 1, 1)], 0, 0, 1, 1),
        0,
        0,
        1,
        1,
      );
      const loopBody = makeStmtBlock([makeStmtExpr(call, null, 0, 0, 1, 1)], 0, 0, 1, 1);
      stmts.push(makeStmtForOf(loopHead, lowerExpr(st, x.arg), loopBody, false, 0, 0, 1, 1));
    } else {
      const pushMem = makeExprMember(arrIdent, makePropName(identNode(PUSH_UNITS), 0, 0, 1, 1), false, 0, 0, 1, 1);
      const call = makeExprCall(pushMem, [lowerExpr(st, x)], false, 0, 0, 1, 1);
      stmts.push(makeStmtExpr(call, null, 0, 0, 1, 1));
    }
  }
  stmts.push(makeStmtReturn(arrIdent, 0, 0, 1, 1));
  return wrapIife(stmts);
}

function lowerMemberProp(st, prop) {
  if (prop.kind === MP_COMPUTED) {
    return makePropComputed(lowerExpr(st, prop.expr), prop.start, prop.end, prop.line, prop.col);
  }
  if (prop.kind === MP_PRIVATE) {
    const privUnits = concatArrays(unitsOf("__priv_"), prop.ident.name);
    return makePropName(identNode(privUnits), prop.start, prop.end, prop.line, prop.col);
  }
  return prop;
}

export function lowerExpr(st, e) {
  if (e === null) {
    return null;
  }
  const k = e.kind;
  if (k === E_THIS) {
    if (st.inArrow) {
      st.arrowUsedThis = true;
      return makeExprIdent(THIS_CAPTURE_UNITS, e.start, e.end, e.line, e.col);
    }
    return e;
  }
  if (k === E_PAREN) {
    return makeExprParen(lowerExpr(st, e.expr), e.start, e.end, e.line, e.col);
  }
  if (k === E_OPT_CHAIN) {
    return lowerExpr(st, e.expr);
  }
  if (k === E_ARRAY) {
    return lowerArrayExpr(st, e);
  }
  if (k === E_OBJECT) {
    return lowerObjectExpr(st, e);
  }
  if (k === E_FUNCTION) {
    return makeExprFunction(lowerFunctionNode(st, e.func), e.start, e.end, e.line, e.col);
  }
  if (k === E_ARROW) {
    return makeExprArrow(lowerArrowNode(st, e.arrow), e.start, e.end, e.line, e.col);
  }
  if (k === E_CLASS) {
    const clsUnits = e.cls.id !== null ? e.cls.id.name : freshUnits(st, "Cls");
    const parts = lowerClassParts(st, e.cls, clsUnits);
    const stmts = [makeStmtClass(parts.cls, e.start, e.end, e.line, e.col)];
    for (const ss of parts.staticStmts) {
      stmts.push(ss);
    }
    stmts.push(makeStmtReturn(identExpr(clsUnits), 0, 0, 1, 1));
    return wrapIife(stmts);
  }
  if (k === E_TEMPLATE) {
    const exprs = [];
    for (const x of e.quasi.exprs) {
      exprs.push(lowerExpr(st, x));
    }
    e.quasi.exprs = exprs;
    return e;
  }
  if (k === E_TAGGED) {
    const rawStrings = [];
    for (const q of e.quasi.quasis) {
      const units = q.cooked !== null ? q.cooked : q.raw;
      rawStrings.push(makeExprStr(units, q.start, q.end, q.line, q.col));
    }
    const callArgs = [makeExprArray(rawStrings, 0, 0, 1, 1)];
    for (const x of e.quasi.exprs) {
      callArgs.push(lowerExpr(st, x));
    }
    return makeExprCall(lowerExpr(st, e.tag), callArgs, false, e.start, e.end, e.line, e.col);
  }
  if (k === E_UNARY) {
    return makeExprUnary(e.op, lowerExpr(st, e.arg), e.start, e.end, e.line, e.col);
  }
  if (k === E_UPDATE) {
    return makeExprUpdate(e.op, e.prefix, lowerExpr(st, e.arg), e.start, e.end, e.line, e.col);
  }
  if (k === E_BINARY) {
    return makeExprBinary(e.op, lowerExpr(st, e.left), lowerExpr(st, e.right), e.start, e.end, e.line, e.col);
  }
  if (k === E_LOGICAL) {
    return makeExprLogical(e.op, lowerExpr(st, e.left), lowerExpr(st, e.right), e.start, e.end, e.line, e.col);
  }
  if (k === E_COND) {
    return makeExprCond(
      lowerExpr(st, e.test),
      lowerExpr(st, e.cons),
      lowerExpr(st, e.alt),
      e.start,
      e.end,
      e.line,
      e.col,
    );
  }
  if (k === E_ASSIGN) {
    const rhs = lowerExpr(st, e.value);
    const t = e.target;
    if (t.kind === PT_ARRAY || t.kind === PT_OBJECT) {
      const rhsUnits = freshUnits(st, "rhs");
      const rhsIdent = identExpr(rhsUnits);
      const stmts = [makeSimpleVarStmt(VK_CONST, rhsUnits, rhs)];
      lowerAssignPattern(st, t, rhsIdent, stmts);
      stmts.push(makeStmtReturn(rhsIdent, 0, 0, 1, 1));
      return wrapIife(stmts);
    }
    let loweredTarget = t;
    if (t.kind === PT_EXPR) {
      loweredTarget = makePatExpr(lowerExpr(st, t.expr), t.start, t.end, t.line, t.col);
    }
    const targetExpr = t.kind === PT_IDENT ? identExpr(t.name) : loweredTarget.expr;
    if (e.op === A_AND) {
      const asg = makeExprAssign(A_ASSIGN, loweredTarget, rhs, e.start, e.end, e.line, e.col);
      return makeExprLogical(L_AND, targetExpr, asg, e.start, e.end, e.line, e.col);
    }
    if (e.op === A_OR) {
      const asg = makeExprAssign(A_ASSIGN, loweredTarget, rhs, e.start, e.end, e.line, e.col);
      return makeExprLogical(L_OR, targetExpr, asg, e.start, e.end, e.line, e.col);
    }
    if (e.op === A_NULLISH) {
      const asg = makeExprAssign(A_ASSIGN, loweredTarget, rhs, e.start, e.end, e.line, e.col);
      return makeExprCond(isNotNullishExpr(targetExpr), targetExpr, asg, e.start, e.end, e.line, e.col);
    }
    return makeExprAssign(e.op, loweredTarget, rhs, e.start, e.end, e.line, e.col);
  }
  if (k === E_MEMBER) {
    const obj = lowerExpr(st, e.object);
    const prop = lowerMemberProp(st, e.prop);
    if (!e.optional) {
      return makeExprMember(obj, prop, false, e.start, e.end, e.line, e.col);
    }
    if (obj.kind === E_IDENT) {
      const access = makeExprMember(obj, prop, false, e.start, e.end, e.line, e.col);
      return makeExprCond(isNullishExpr(obj), undefExpr(), access, e.start, e.end, e.line, e.col);
    }
    const tmpUnits = freshUnits(st, "oc");
    const tmpIdent = identExpr(tmpUnits);
    const access = makeExprMember(tmpIdent, prop, false, e.start, e.end, e.line, e.col);
    const cond = makeExprCond(isNullishExpr(tmpIdent), undefExpr(), access, e.start, e.end, e.line, e.col);
    return wrapArrowOne(tmpUnits, cond, obj);
  }
  if (k === E_CALL) {
    const callee = lowerExpr(st, e.callee);
    const args = [];
    for (const a of e.args) {
      if (a.kind === E_SPREAD) {
        args.push(lowerExpr(st, a.arg));
      } else {
        args.push(lowerExpr(st, a));
      }
    }
    if (!e.optional) {
      return makeExprCall(callee, args, false, e.start, e.end, e.line, e.col);
    }
    if (callee.kind === E_IDENT) {
      const call = makeExprCall(callee, args, false, e.start, e.end, e.line, e.col);
      return makeExprCond(isNullishExpr(callee), undefExpr(), call, e.start, e.end, e.line, e.col);
    }
    const tmpUnits = freshUnits(st, "fn");
    const tmpIdent = identExpr(tmpUnits);
    const call = makeExprCall(tmpIdent, args, false, e.start, e.end, e.line, e.col);
    const cond = makeExprCond(isNullishExpr(tmpIdent), undefExpr(), call, e.start, e.end, e.line, e.col);
    return wrapArrowOne(tmpUnits, cond, callee);
  }
  if (k === E_NEW) {
    const callee = lowerExpr(st, e.callee);
    const args = [];
    for (const a of e.args) {
      if (a.kind === E_SPREAD) {
        args.push(lowerExpr(st, a.arg));
      } else {
        args.push(lowerExpr(st, a));
      }
    }
    return makeExprNew(callee, args, e.start, e.end, e.line, e.col);
  }
  if (k === E_SEQ) {
    const items = [];
    for (const x of e.items) {
      items.push(lowerExpr(st, x));
    }
    return makeExprSeq(items, e.start, e.end, e.line, e.col);
  }
  if (k === E_AWAIT || k === E_SPREAD) {
    return lowerExpr(st, e.arg);
  }
  if (k === E_YIELD) {
    if (e.arg === null) {
      return undefExpr();
    }
    return lowerExpr(st, e.arg);
  }
  if (k === E_PRIVATE_IN) {
    const privUnits = concatArrays(unitsOf("__priv_"), e.name.name);
    return makeExprBinary(
      B_IN,
      makeExprStr(privUnits, e.name.start, e.name.end, e.name.line, e.name.col),
      lowerExpr(st, e.right),
      e.start,
      e.end,
      e.line,
      e.col,
    );
  }
  return e;
}

function lowerVarDeclToStmts(st, vd, outStmts) {
  for (const d of vd.decls) {
    const initExpr = d.init !== null ? lowerExpr(st, d.init) : null;
    if (d.target.kind === PT_IDENT) {
      const decl = makeVarDeclarator(d.target, initExpr, d.start, d.end, d.line, d.col);
      const single = makeVarDecl(vd.varKind, [decl], d.start, d.end, d.line, d.col);
      outStmts.push(makeStmtVar(single, d.start, d.end, d.line, d.col));
    } else {
      const rhs = initExpr !== null ? initExpr : undefExpr();
      lowerBindingPattern(st, d.target, rhs, vd.varKind, outStmts);
    }
  }
}

function lowerStmtInto(st, s, outStmts) {
  const k = s.kind;
  if (k === S_EXPR) {
    outStmts.push(makeStmtExpr(lowerExpr(st, s.expr), s.directive, s.start, s.end, s.line, s.col));
    return;
  }
  if (k === S_VAR) {
    lowerVarDeclToStmts(st, s.decl, outStmts);
    return;
  }
  if (k === S_FUNCTION) {
    const lf = lowerFunctionNode(st, s.func);
    if (st.blockDepth > 0 && lf.id !== null) {
      const fnExpr = makeExprFunction(lf, s.start, s.end, s.line, s.col);
      outStmts.push(makeSimpleVarStmt(VK_LET, lf.id.name, fnExpr));
    } else {
      outStmts.push(makeStmtFunction(lf, s.start, s.end, s.line, s.col));
    }
    return;
  }
  if (k === S_CLASS) {
    const clsUnits = s.cls.id !== null ? s.cls.id.name : freshUnits(st, "Cls");
    const parts = lowerClassParts(st, s.cls, clsUnits);
    outStmts.push(makeStmtClass(parts.cls, s.start, s.end, s.line, s.col));
    for (const ss of parts.staticStmts) {
      outStmts.push(ss);
    }
    return;
  }
  if (k === S_BLOCK) {
    st.blockDepth += 1;
    const body = lowerStmtList(st, s.body);
    st.blockDepth -= 1;
    outStmts.push(makeStmtBlock(body, s.start, s.end, s.line, s.col));
    return;
  }
  if (k === S_IF) {
    st.blockDepth += 1;
    const cons = lowerSingleStmtAsBlock(st, s.cons);
    const alt = s.alt !== null ? lowerSingleStmt(st, s.alt) : null;
    st.blockDepth -= 1;
    outStmts.push(makeStmtIf(lowerExpr(st, s.test), cons, alt, s.start, s.end, s.line, s.col));
    return;
  }
  if (k === S_WHILE) {
    st.blockDepth += 1;
    const body = lowerSingleStmtAsBlock(st, s.body);
    st.blockDepth -= 1;
    outStmts.push(makeStmtWhile(lowerExpr(st, s.test), body, s.start, s.end, s.line, s.col));
    return;
  }
  if (k === S_DO_WHILE) {
    st.blockDepth += 1;
    const body = lowerSingleStmtAsBlock(st, s.body);
    st.blockDepth -= 1;
    outStmts.push(makeStmtDoWhile(body, lowerExpr(st, s.test), s.start, s.end, s.line, s.col));
    return;
  }
  if (k === S_FOR) {
    st.blockDepth += 1;
    let init = null;
    const preStmts = [];
    if (s.init !== null) {
      if (s.init.kind === FI_VAR) {
        const decls = s.init.decl.decls;
        if (decls.length === 1 && decls[0].target.kind === PT_IDENT) {
          const d0 = decls[0];
          const initE = d0.init !== null ? lowerExpr(st, d0.init) : null;
          const vd = makeVarDecl(
            s.init.decl.varKind,
            [makeVarDeclarator(d0.target, initE, d0.start, d0.end, d0.line, d0.col)],
            s.init.decl.start,
            s.init.decl.end,
            s.init.decl.line,
            s.init.decl.col,
          );
          init = makeForInitVar(vd, s.init.start, s.init.end, s.init.line, s.init.col);
        } else {
          lowerVarDeclToStmts(st, s.init.decl, preStmts);
        }
      } else {
        init = makeForInitExpr(lowerExpr(st, s.init.expr), s.init.start, s.init.end, s.init.line, s.init.col);
      }
    }
    const test = s.test !== null ? lowerExpr(st, s.test) : null;
    const update = s.update !== null ? lowerExpr(st, s.update) : null;
    const body = lowerSingleStmtAsBlock(st, s.body);
    st.blockDepth -= 1;
    const forStmt = makeStmtFor(init, test, update, body, s.start, s.end, s.line, s.col);
    if (preStmts.length > 0) {
      preStmts.push(forStmt);
      outStmts.push(makeStmtBlock(preStmts, s.start, s.end, s.line, s.col));
    } else {
      outStmts.push(forStmt);
    }
    return;
  }
  if (k === S_FOR_OF || k === S_FOR_IN) {
    st.blockDepth += 1;
    const rawRight = lowerExpr(st, s.right);
    const right =
      k === S_FOR_IN
        ? makeExprCall(identExpr(ANVIL_KEYS_UNITS), [rawRight], false, 0, 0, 1, 1)
        : rawRight;
    const bodyBlock = lowerSingleStmtAsBlock(st, s.body);
    st.blockDepth -= 1;

    if (s.left.kind === FH_VAR) {
      const d = s.left.decl;
      const target = d.decls[0].target;
      if (target.kind === PT_IDENT) {
        outStmts.push(makeStmtForOf(s.left, right, bodyBlock, false, s.start, s.end, s.line, s.col));
        return;
      }
      const itemUnits = freshUnits(st, "item");
      const headDecl = makeVarDecl(
        d.varKind,
        [makeVarDeclarator(patIdent(itemUnits), null, 0, 0, 1, 1)],
        0,
        0,
        1,
        1,
      );
      const head = makeForHeadVar(headDecl, 0, 0, 1, 1);
      const unpackStmts = [];
      lowerBindingPattern(st, target, identExpr(itemUnits), d.varKind, unpackStmts);
      const newBody = makeStmtBlock(concatArrays(unpackStmts, bodyBlock.body), bodyBlock.start, bodyBlock.end, bodyBlock.line, bodyBlock.col);
      outStmts.push(makeStmtForOf(head, right, newBody, false, s.start, s.end, s.line, s.col));
      return;
    }
    // `for (x of right)` where `x` is a pattern or existing variable.
    const itemUnits = freshUnits(st, "item");
    const headDecl = makeVarDecl(
      VK_CONST,
      [makeVarDeclarator(patIdent(itemUnits), null, 0, 0, 1, 1)],
      0,
      0,
      1,
      1,
    );
    const head = makeForHeadVar(headDecl, 0, 0, 1, 1);
    const assignStmts = [];
    lowerAssignPattern(st, s.left.pat, identExpr(itemUnits), assignStmts);
    const newBody = makeStmtBlock(concatArrays(assignStmts, bodyBlock.body), bodyBlock.start, bodyBlock.end, bodyBlock.line, bodyBlock.col);
    outStmts.push(makeStmtForOf(head, right, newBody, false, s.start, s.end, s.line, s.col));
    return;
  }
  if (k === S_RETURN) {
    const arg = s.arg !== null ? lowerExpr(st, s.arg) : null;
    outStmts.push(makeStmtReturn(arg, s.start, s.end, s.line, s.col));
    return;
  }
  if (k === S_THROW) {
    outStmts.push(makeStmtThrow(lowerExpr(st, s.arg), s.start, s.end, s.line, s.col));
    return;
  }
  if (k === S_TRY) {
    st.blockDepth += 1;
    const tryBlock = lowerStmtList(st, s.block);
    let handler = null;
    if (s.handler !== null) {
      const h = s.handler;
      const catchBody = lowerStmtList(st, h.body);
      if (h.param === null) {
        const errUnits = freshUnits(st, "err");
        handler = makeCatch(patIdent(errUnits), catchBody, h.start, h.end, h.line, h.col);
      } else if (h.param.kind === PT_IDENT) {
        handler = makeCatch(h.param, catchBody, h.start, h.end, h.line, h.col);
      } else {
        const errUnits = freshUnits(st, "err");
        const unpack = [];
        lowerBindingPattern(st, h.param, identExpr(errUnits), VK_LET, unpack);
        handler = makeCatch(patIdent(errUnits), concatArrays(unpack, catchBody), h.start, h.end, h.line, h.col);
      }
    }
    const finalizer = s.finalizer !== null ? lowerStmtList(st, s.finalizer) : null;
    st.blockDepth -= 1;
    outStmts.push(makeStmtTry(tryBlock, handler, finalizer, s.start, s.end, s.line, s.col));
    return;
  }
  if (k === S_SWITCH) {
    st.blockDepth += 1;
    const cases = [];
    for (const c of s.cases) {
      const test = c.test !== null ? lowerExpr(st, c.test) : null;
      const body = lowerStmtList(st, c.body);
      cases.push(makeSwitchCase(test, body, c.start, c.end, c.line, c.col));
    }
    st.blockDepth -= 1;
    outStmts.push(makeStmtSwitch(lowerExpr(st, s.disc), cases, s.start, s.end, s.line, s.col));
    return;
  }
  if (k === S_LABELED) {
    outStmts.push(makeStmtLabeled(s.label, lowerSingleStmt(st, s.body), s.start, s.end, s.line, s.col));
    return;
  }
  if (k === S_IMPORT) {
    return;
  }
  if (k === S_EXPORT) {
    const d = s.decl;
    if (d.kind === ED_DECL) {
      lowerStmtInto(st, d.stmt, outStmts);
    } else if (d.kind === ED_DEFAULT) {
      const x = d.def;
      if (x.kind === EX_FUNCTION) {
        outStmts.push(makeStmtFunction(lowerFunctionNode(st, x.func), s.start, s.end, s.line, s.col));
      } else if (x.kind === EX_CLASS) {
        const clsUnits = x.cls.id !== null ? x.cls.id.name : freshUnits(st, "DefaultCls");
        const parts = lowerClassParts(st, x.cls, clsUnits);
        outStmts.push(makeStmtClass(parts.cls, s.start, s.end, s.line, s.col));
        for (const ss of parts.staticStmts) {
          outStmts.push(ss);
        }
      } else {
        outStmts.push(makeStmtExpr(lowerExpr(st, x.expr), null, s.start, s.end, s.line, s.col));
      }
    }
    return;
  }
  outStmts.push(s);
}

function lowerStmtList(st, stmts) {
  const out = [];
  for (const s of stmts) {
    lowerStmtInto(st, s, out);
  }
  return out;
}

function lowerSingleStmt(st, s) {
  const out = [];
  lowerStmtInto(st, s, out);
  if (out.length === 0) {
    return makeStmtEmpty(s.start, s.end, s.line, s.col);
  }
  if (out.length === 1) {
    return out[0];
  }
  return makeStmtBlock(out, s.start, s.end, s.line, s.col);
}

function lowerSingleStmtAsBlock(st, s) {
  if (s.kind === S_BLOCK) {
    return makeStmtBlock(lowerStmtList(st, s.body), s.start, s.end, s.line, s.col);
  }
  const out = [];
  lowerStmtInto(st, s, out);
  return makeStmtBlock(out, s.start, s.end, s.line, s.col);
}

export function lowerProgram(prog) {
  const st = newState();
  const body = lowerStmtList(st, prog.body);
  return makeProgram(false, prog.strict, body, prog.start, prog.end, prog.line, prog.col);
}
