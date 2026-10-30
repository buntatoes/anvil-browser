// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Scope analysis of a parsed program (spec 9.1 Environment Records, 10.2.11
// FunctionDeclarationInstantiation, 14.2.3 BlockDeclarationInstantiation,
// 16.1.7 GlobalDeclarationInstantiation, 16.2.1.6.4 module environments,
// 14.7.4.2 and 14.7.5.6 loop environments, 15.7.14 class environments,
// and annex B.3.2 for functions in blocks of sloppy code).
//
// analyzeProgram(prog) returns an analysis record: the scope tree, every
// binding, every function, and every identifier reference resolved to a
// binding or marked global, with the facts the lowering step needs
// (hoisting, TDZ checks, captures, `arguments`/`this`/`new.target`/`super`
// use, fresh loop bindings, strictness, annex B decisions). The layout is
// described in docs/SCOPE.md.
//
// The program must have passed the parser, early errors included: the
// analysis does not report errors. It walks the tree once, creating
// scopes and declaring names as it goes and noting each reference, then
// decides the annex B copies and the `arguments` objects, and resolves
// every reference at the end, when every scope is complete.

import { newNameMap, nameMapGet, nameMapSet, sameUnits } from "./names.js";
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
  S_WITH,
  S_SWITCH,
  S_LABELED,
  S_THROW,
  S_TRY,
  S_IMPORT,
  S_EXPORT,
  FI_VAR,
  FH_VAR,
  AB_EXPR,
  CM_METHOD,
  CM_FIELD,
  PK_COMPUTED,
  PK_PRIVATE,
  E_IDENT,
  E_THIS,
  E_SUPER,
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
  E_NEW_TARGET,
  E_IMPORT,
  E_PAREN,
  E_PRIVATE_IN,
  MP_COMPUTED,
  MP_PRIVATE,
  PR_INIT,
  PR_SHORTHAND,
  PR_METHOD,
  PT_IDENT,
  PT_ARRAY,
  PT_OBJECT,
  PT_ASSIGN,
  PT_REST,
  PP_PROP,
  IS_NAMED,
  IS_NAMESPACE,
  ED_NAMED,
  ED_DECL,
  ED_DEFAULT,
  EX_FUNCTION,
  EX_CLASS,
  MN_IDENT,
  MK_GET,
  MK_SET,
  MK_CONSTRUCTOR,
  U_DELETE,
  U_TYPEOF,
  A_ASSIGN,
  VK_VAR,
  VK_CONST,
} from "./ast.js";

// ---- kinds ----

// Scopes.
export const SC_GLOBAL = 0; // the global environment (script top level)
export const SC_MODULE = 1; // a module's top level
export const SC_FUNCTION = 2; // parameters (and the body, see SC_BODY)
export const SC_BODY = 3; // a body apart from parameters with expressions
export const SC_FUNC_NAME = 4; // a named function expression's own name
export const SC_BLOCK = 5; // a block, a `try`/`finally` block, a catch body
export const SC_CATCH = 6; // a catch parameter
export const SC_CLASS = 7; // a class's inner name and heritage
export const SC_CLASS_BODY = 8; // a class body: private names, members
export const SC_FOR = 9; // the `let`/`const` head of a `for` loop
export const SC_SWITCH = 10; // the cases of a `switch`
export const SC_WITH = 11; // a `with` body: names may come from the object
export const SCOPE_KIND_COUNT = 12;

// Bindings.
export const B_VAR = 0;
export const B_LET = 1;
export const B_CONST = 2;
export const B_CLASS = 3; // a class declaration, in the enclosing scope
export const B_FUNCTION = 4;
export const B_PARAM = 5;
export const B_CATCH = 6;
export const B_IMPORT = 7;
export const B_ARGUMENTS = 8; // the implicit `arguments` object
export const B_FUNC_NAME = 9; // a function expression's own name
export const B_CLASS_NAME = 10; // a class's inner name
export const B_PRIVATE = 11; // a private name of a class body
export const BINDING_KIND_COUNT = 12;

// Private name kinds (binding.privateKind).
export const PN_FIELD = 0;
export const PN_METHOD = 1;
export const PN_GET = 2;
export const PN_SET = 3;
export const PN_ACCESSOR = 4; // a getter and a setter

// Functions (the program itself is function 0).
export const FN_SCRIPT = 0;
export const FN_MODULE = 1;
export const FN_FUNCTION = 2; // declarations and expressions
export const FN_ARROW = 3;
export const FN_METHOD = 4; // class and object methods
export const FN_GETTER = 5;
export const FN_SETTER = 6;
export const FN_CONSTRUCTOR = 7;
export const FN_FIELD = 8; // a class field initializer
export const FN_STATIC_BLOCK = 9;
export const FUNC_KIND_COUNT = 10;

// References.
export const R_READ = 0;
export const R_WRITE = 1;
export const R_READ_WRITE = 2; // compound assignment, `++`, `--`
export const R_EXPORT = 3; // `export { x }`: the binding is exported
export const R_PRIVATE = 4; // `#x` in `o.#x` or `#x in o`

// How a reference resolved.
export const RES_LOCAL = 0; // a binding of the same function
export const RES_CAPTURED = 1; // a binding of an enclosing function
export const RES_GLOBAL = 2; // no binding: the global object, or an error

// `*default*`, the name `export default` binds (16.2.3.7).
// A declEnd no reference reaches: every reference needs a TDZ check.
const ALWAYS_CHECK = 2147483647;

export const DEFAULT_NAME = [0x2a, 0x64, 0x65, 0x66, 0x61, 0x75, 0x6c, 0x74, 0x2a];
const ARGUMENTS = [0x61, 0x72, 0x67, 0x75, 0x6d, 0x65, 0x6e, 0x74, 0x73];
const EVAL = [0x65, 0x76, 0x61, 0x6c];

// ---- records ----

function newAnalysis(isModule) {
  return {
    isModule: isModule,
    scopes: [],
    bindings: [],
    funcs: [],
    refs: [],
    // Function declarations in blocks that annex B may copy to a `var`.
    annexCandidates: [],
    // Identifier nodes that declare a binding, with the binding id.
    declNodes: [],
    declBindings: [],
  };
}

function newScope(a, kind, parent, func, node, strict) {
  const s = {
    id: a.scopes.length,
    kind: kind,
    parent: parent,
    func: func,
    node: node,
    strict: strict,
    bindings: [],
    names: newNameMap(),
    // Private names (class bodies only).
    privates: null,
    children: [],
    // Function declarations instantiated when the scope is entered, in
    // order; for a repeated name only the last one.
    funcInits: [],
    // A sloppy direct eval may add `var` bindings here at run time.
    grows: false,
    // A direct eval can see this scope's bindings by name.
    evalVisible: false,
  };
  a.scopes.push(s);
  if (parent >= 0) {
    a.scopes[parent].children.push(s.id);
  }
  return s.id;
}

function newBinding(a, scope, name, kind, decl, declEnd, tdz) {
  const b = {
    id: a.bindings.length,
    name: name,
    kind: kind,
    scope: scope,
    // The node of the (first) declaration: an identifier, or the function
    // or statement for implicit bindings.
    decl: decl,
    // References in the same function at or after this offset need no
    // TDZ check (except in a `switch`).
    declEnd: declEnd,
    tdz: tdz,
    captured: false,
    // Functions other than the binding's own that reference it, nearest
    // first per reference.
    capturedBy: [],
    written: false,
    refCount: 0,
    // Captured and written: it has to live in a shared cell.
    mutatedAfterCapture: false,
    // A loop binding that needs a fresh copy per iteration.
    loopFresh: false,
    exported: false,
    // Annex B: the `var` binding a block function is copied to (-1: none).
    annexVar: -1,
    // A body `var` that starts with the value of this parameter, or of
    // the arguments object (-1: none).
    fromParam: -1,
    privateKind: -1,
    isStatic: false,
    // Made only for an annex B copy (no `var` of the name was written).
    annexOnly: false,
    // Holds the function's arguments object (its own binding, or a `var
    // arguments` next to the parameters).
    argumentsObject: false,
  };
  a.bindings.push(b);
  a.scopes[scope].bindings.push(b.id);
  nameMapSet(a.scopes[scope].names, name, b.id);
  return b.id;
}

function newFunc(a, kind, node, parent, strict) {
  const f = {
    id: a.funcs.length,
    kind: kind,
    node: node,
    parent: parent,
    // Its outermost scope (parameters), and the scope `var`s go to.
    scope: -1,
    varScope: -1,
    strict: strict,
    arrow: kind === FN_ARROW,
    simpleParams: true,
    paramExpressions: false,
    paramNames: newNameMap(),
    // Uses, own or from arrows (and direct evals) inside.
    usesThis: false,
    usesNewTarget: false,
    usesSuperProperty: false,
    usesSuperCall: false,
    mentionsArguments: false,
    // An arrow inside uses this function's `this` (through `this`,
    // `super` or a direct eval), and through `this` itself.
    thisFromArrows: false,
    thisWordFromArrows: false,
    // A direct eval call in this function itself, and whether it is
    // sloppy; evalInside: in it or in an arrow it contains.
    directEval: false,
    sloppyEval: false,
    evalInside: false,
    argumentsBinding: -1,
    usesArguments: false,
    mapped: false,
    // Bindings of enclosing functions this function (or a function inside
    // it) references.
    captures: [],
  };
  a.funcs.push(f);
  return f.id;
}

// The nearest function that has its own `this` (not an arrow).
function thisFunc(a, s) {
  let f = a.scopes[s].func;
  while (a.funcs[f].arrow) {
    f = a.funcs[f].parent;
  }
  return f;
}

function lookupHere(a, s, name) {
  const v = nameMapGet(a.scopes[s].names, name);
  if (v === null) {
    return -1;
  }
  return v;
}

function noteDecl(a, node, b) {
  a.declNodes.push(node);
  a.declBindings.push(b);
}

function addRef(a, s, name, node, kind) {
  const r = {
    id: a.refs.length,
    name: name,
    node: node,
    kind: kind,
    scope: s,
    func: a.scopes[s].func,
    binding: -1,
    resolution: RES_GLOBAL,
    // A `with` object or a sloppy direct eval may supply the name first.
    dynamic: false,
    tdzCheck: false,
    typeofRef: false,
    deleteRef: false,
    // The write of a `var` declaration's initializer or loop head.
    declInit: false,
  };
  a.refs.push(r);
  if (kind !== R_PRIVATE && sameUnits(name, ARGUMENTS)) {
    a.funcs[thisFunc(a, s)].mentionsArguments = true;
  }
  return r;
}

// ---- declarations ----

function varScopeOf(a, s) {
  return a.funcs[a.scopes[s].func].varScope;
}

// A `var` name (16.1.7, 10.2.11 steps 27 and 28): one binding per name in
// the var scope; a body var scope separate from parameters starts a new
// binding initialised from a parameter of the same name.
function declareVarName(a, s, id) {
  const vs = varScopeOf(a, s);
  let b = lookupHere(a, vs, id.name);
  if (b < 0) {
    b = newBinding(a, vs, id.name, B_VAR, id, 0, false);
    const sc = a.scopes[vs];
    if (sc.kind === SC_BODY) {
      const p = lookupHere(a, sc.parent, id.name);
      if (p >= 0) {
        a.bindings[b].fromParam = p;
      }
    }
  }
  noteDecl(a, id, b);
  return b;
}

function declareLexicalName(a, s, id, kind, declEnd) {
  const b = newBinding(a, s, id.name, kind, id, declEnd, true);
  noteDecl(a, id, b);
  return b;
}

// Bound names of a binding pattern, in order.
function patNames(p, out) {
  const k = p.kind;
  if (k === PT_IDENT) {
    out.push(p);
  } else if (k === PT_ARRAY) {
    let i = 0;
    while (i < p.items.length) {
      if (p.items[i] !== null) {
        patNames(p.items[i], out);
      }
      i = i + 1;
    }
  } else if (k === PT_OBJECT) {
    let i = 0;
    while (i < p.props.length) {
      const pp = p.props[i];
      if (pp.kind === PP_PROP) {
        patNames(pp.value, out);
      } else {
        patNames(pp.arg, out);
      }
      i = i + 1;
    }
  } else if (k === PT_ASSIGN) {
    patNames(p.target, out);
  } else if (k === PT_REST) {
    patNames(p.arg, out);
  }
}

// ContainsExpression (15.1.2): a default value or a computed key.
function patHasExpression(p) {
  const k = p.kind;
  if (k === PT_ASSIGN) {
    return true;
  }
  if (k === PT_ARRAY) {
    let i = 0;
    while (i < p.items.length) {
      if (p.items[i] !== null && patHasExpression(p.items[i])) {
        return true;
      }
      i = i + 1;
    }
    return false;
  }
  if (k === PT_OBJECT) {
    let i = 0;
    while (i < p.props.length) {
      const pp = p.props[i];
      if (pp.kind === PP_PROP) {
        if (pp.key.kind === PK_COMPUTED || patHasExpression(pp.value)) {
          return true;
        }
      } else if (patHasExpression(pp.arg)) {
        return true;
      }
      i = i + 1;
    }
    return false;
  }
  if (k === PT_REST) {
    return patHasExpression(p.arg);
  }
  return false;
}

// Instantiate function declaration fn at entry of scope s: a later one of
// the same name replaces an earlier one.
function addFuncInit(a, s, fn) {
  const old = a.scopes[s].funcInits;
  const list = [];
  let i = 0;
  while (i < old.length) {
    if (old[i].id === null || !sameUnits(old[i].id.name, fn.id.name)) {
      list.push(old[i]);
    }
    i = i + 1;
  }
  list.push(fn);
  a.scopes[s].funcInits = list;
}

// A function declaration. In a var scope it is var-like (and at a
// module's top level lexical but initialised at once); in a block it is
// lexical, initialised when the block is entered.
function declareFunctionDecl(a, s, fn, id, annexOk) {
  const vs = varScopeOf(a, s);
  let b = -1;
  if (s === vs) {
    b = lookupHere(a, s, id.name);
    if (b < 0) {
      b = newBinding(a, s, id.name, B_FUNCTION, id, 0, false);
      const sc = a.scopes[s];
      if (sc.kind === SC_BODY) {
        const p = lookupHere(a, sc.parent, id.name);
        if (p >= 0) {
          a.bindings[b].fromParam = p;
        }
      }
    } else if (a.bindings[b].kind === B_VAR) {
      a.bindings[b].kind = B_FUNCTION;
    }
  } else {
    b = lookupHere(a, s, id.name);
    if (b < 0) {
      b = newBinding(a, s, id.name, B_FUNCTION, id, 0, false);
    }
    if (annexOk && !a.scopes[s].strict && !fn.isAsync && !fn.isGenerator) {
      a.annexCandidates.push(b);
    }
  }
  noteDecl(a, id, b);
  addFuncInit(a, s, fn);
  return b;
}

// ---- walking ----

export function analyzeProgram(prog) {
  const a = newAnalysis(prog.module);
  let kind = FN_SCRIPT;
  if (prog.module) {
    kind = FN_MODULE;
  }
  const f = newFunc(a, kind, prog, -1, prog.strict);
  const g = newScope(a, SC_GLOBAL, -1, f, prog, prog.strict);
  let top = g;
  if (prog.module) {
    top = newScope(a, SC_MODULE, g, f, prog, true);
  }
  a.funcs[f].scope = top;
  a.funcs[f].varScope = top;
  walkStmts(a, top, prog.body);
  decideAnnex(a);
  decideArguments(a);
  resolveAll(a);
  finish(a);
  return a;
}

function walkStmts(a, s, list) {
  let i = 0;
  while (i < list.length) {
    walkStmt(a, s, list[i]);
    i = i + 1;
  }
}

function walkStmt(a, s, st) {
  const k = st.kind;
  if (k === S_EXPR || k === S_THROW) {
    if (k === S_EXPR) {
      walkExpr(a, s, st.expr);
    } else {
      walkExpr(a, s, st.arg);
    }
  } else if (k === S_VAR) {
    walkVarDecl(a, s, st.decl, -1);
  } else if (k === S_FUNCTION) {
    declareFunctionDecl(a, s, st.func, st.func.id, true);
    walkFunction(a, s, st.func, FN_FUNCTION, false);
  } else if (k === S_CLASS) {
    walkClass(a, s, st.cls, true);
  } else if (k === S_BLOCK) {
    const b = newScope(a, SC_BLOCK, s, a.scopes[s].func, st, a.scopes[s].strict);
    walkStmts(a, b, st.body);
  } else if (k === S_IF) {
    walkExpr(a, s, st.test);
    walkBranch(a, s, st.cons);
    if (st.alt !== null) {
      walkBranch(a, s, st.alt);
    }
  } else if (k === S_FOR) {
    walkFor(a, s, st);
  } else if (k === S_FOR_IN || k === S_FOR_OF) {
    walkForEach(a, s, st);
  } else if (k === S_WHILE || k === S_DO_WHILE) {
    walkExpr(a, s, st.test);
    walkStmt(a, s, st.body);
  } else if (k === S_RETURN) {
    if (st.arg !== null) {
      walkExpr(a, s, st.arg);
    }
  } else if (k === S_WITH) {
    walkExpr(a, s, st.object);
    const w = newScope(a, SC_WITH, s, a.scopes[s].func, st, a.scopes[s].strict);
    walkStmt(a, w, st.body);
  } else if (k === S_SWITCH) {
    walkExpr(a, s, st.disc);
    const sw = newScope(a, SC_SWITCH, s, a.scopes[s].func, st, a.scopes[s].strict);
    let i = 0;
    while (i < st.cases.length) {
      const c = st.cases[i];
      if (c.test !== null) {
        walkExpr(a, sw, c.test);
      }
      walkStmts(a, sw, c.body);
      i = i + 1;
    }
  } else if (k === S_LABELED) {
    let body = st.body;
    while (body.kind === S_LABELED) {
      body = body.body;
    }
    if (body.kind === S_FUNCTION) {
      // A labelled function declaration is declared like any other, but
      // annex B.3.2.1 covers only functions directly in a statement list.
      declareFunctionDecl(a, s, body.func, body.func.id, false);
      walkFunction(a, s, body.func, FN_FUNCTION, false);
    } else {
      walkStmt(a, s, body);
    }
  } else if (k === S_TRY) {
    walkTry(a, s, st);
  } else if (k === S_IMPORT) {
    walkImport(a, s, st.decl);
  } else if (k === S_EXPORT) {
    walkExport(a, s, st);
  }
}

// An `if` branch: a function declaration there (annex B.3.3) sits in a
// block of its own.
function walkBranch(a, s, st) {
  if (st.kind === S_FUNCTION) {
    const b = newScope(a, SC_BLOCK, s, a.scopes[s].func, st, a.scopes[s].strict);
    walkStmt(a, b, st);
  } else {
    walkStmt(a, s, st);
  }
}

// `var`, `let` or `const` declarations. declEndAt >= 0 overrides where
// lexical bindings count as initialised (loop heads).
function walkVarDecl(a, s, d, declEndAt) {
  let i = 0;
  while (i < d.decls.length) {
    const x = d.decls[i];
    const names = [];
    patNames(x.target, names);
    let end = x.end;
    if (declEndAt >= 0) {
      end = declEndAt;
    }
    let j = 0;
    while (j < names.length) {
      if (d.varKind === VK_VAR) {
        declareVarName(a, s, names[j]);
      } else {
        let kind = B_LET;
        if (d.varKind === VK_CONST) {
          kind = B_CONST;
        }
        declareLexicalName(a, s, names[j], kind, end);
      }
      j = j + 1;
    }
    if (x.init !== null) {
      walkExpr(a, s, x.init);
    }
    walkBindingPat(a, s, x.target, d.varKind === VK_VAR && x.init !== null);
    i = i + 1;
  }
}

// The expressions inside a binding pattern (defaults, computed keys);
// with varWrite, each name is also written by name, as a `var`
// initializer is (14.3.2.1: PutValue on the resolved reference).
function walkBindingPat(a, s, p, varWrite) {
  const k = p.kind;
  if (k === PT_IDENT) {
    if (varWrite) {
      const r = addRef(a, s, p.name, p, R_WRITE);
      r.declInit = true;
    }
  } else if (k === PT_ARRAY) {
    let i = 0;
    while (i < p.items.length) {
      if (p.items[i] !== null) {
        walkBindingPat(a, s, p.items[i], varWrite);
      }
      i = i + 1;
    }
  } else if (k === PT_OBJECT) {
    let i = 0;
    while (i < p.props.length) {
      const pp = p.props[i];
      if (pp.kind === PP_PROP) {
        if (pp.key.kind === PK_COMPUTED) {
          walkExpr(a, s, pp.key.expr);
        }
        walkBindingPat(a, s, pp.value, varWrite);
      } else {
        walkBindingPat(a, s, pp.arg, varWrite);
      }
      i = i + 1;
    }
  } else if (k === PT_ASSIGN) {
    walkBindingPat(a, s, p.target, varWrite);
    walkExpr(a, s, p.def);
  } else if (k === PT_REST) {
    walkBindingPat(a, s, p.arg, varWrite);
  }
}

function stripParens(e) {
  let x = e;
  while (x.kind === E_PAREN) {
    x = x.expr;
  }
  return x;
}

// An assignment target (13.15.5 destructuring assignment).
function walkTarget(a, s, p, kind) {
  const k = p.kind;
  if (k === PT_IDENT) {
    addRef(a, s, p.name, p, kind);
  } else if (k === PT_ARRAY) {
    let i = 0;
    while (i < p.items.length) {
      if (p.items[i] !== null) {
        walkTarget(a, s, p.items[i], R_WRITE);
      }
      i = i + 1;
    }
  } else if (k === PT_OBJECT) {
    let i = 0;
    while (i < p.props.length) {
      const pp = p.props[i];
      if (pp.kind === PP_PROP) {
        if (pp.key.kind === PK_COMPUTED) {
          walkExpr(a, s, pp.key.expr);
        }
        walkTarget(a, s, pp.value, R_WRITE);
      } else {
        walkTarget(a, s, pp.arg, R_WRITE);
      }
      i = i + 1;
    }
  } else if (k === PT_ASSIGN) {
    walkTarget(a, s, p.target, R_WRITE);
    walkExpr(a, s, p.def);
  } else if (k === PT_REST) {
    walkTarget(a, s, p.arg, R_WRITE);
  } else {
    walkExprTarget(a, s, p.expr, kind);
  }
}

function walkExprTarget(a, s, e, kind) {
  const x = stripParens(e);
  if (x.kind === E_IDENT) {
    addRef(a, s, x.name, x, kind);
  } else {
    walkExpr(a, s, x);
  }
}

function walkFor(a, s, st) {
  let inner = s;
  const init = st.init;
  if (init !== null && init.kind === FI_VAR && init.decl.varKind !== VK_VAR) {
    inner = newScope(a, SC_FOR, s, a.scopes[s].func, st, a.scopes[s].strict);
  }
  if (init !== null) {
    if (init.kind === FI_VAR) {
      walkVarDecl(a, inner, init.decl, -1);
    } else {
      walkExpr(a, inner, init.expr);
    }
  }
  if (st.test !== null) {
    walkExpr(a, inner, st.test);
  }
  if (st.update !== null) {
    walkExpr(a, inner, st.update);
  }
  walkStmt(a, inner, st.body);
}

// for-in and for-of (14.7.5.6, 14.7.5.7): with a `let`/`const` head the
// names are in TDZ while the object expression is evaluated, and every
// iteration gets fresh bindings.
function walkForEach(a, s, st) {
  const left = st.left;
  if (left.kind === FH_VAR && left.decl.varKind !== VK_VAR) {
    const f = newScope(a, SC_FOR, s, a.scopes[s].func, st, a.scopes[s].strict);
    walkVarDecl(a, f, left.decl, st.right.end);
    walkExpr(a, f, st.right);
    walkStmt(a, f, st.body);
    return;
  }
  walkExpr(a, s, st.right);
  if (left.kind === FH_VAR) {
    const d = left.decl;
    let i = 0;
    while (i < d.decls.length) {
      const x = d.decls[i];
      const names = [];
      patNames(x.target, names);
      let j = 0;
      while (j < names.length) {
        declareVarName(a, s, names[j]);
        j = j + 1;
      }
      if (x.init !== null) {
        // Annex B.3.5: `for (var x = e in o)`.
        walkExpr(a, s, x.init);
      }
      walkBindingPat(a, s, x.target, true);
      i = i + 1;
    }
  } else {
    walkTarget(a, s, left.pat, R_WRITE);
  }
  walkStmt(a, s, st.body);
}

function walkTry(a, s, st) {
  const func = a.scopes[s].func;
  const strict = a.scopes[s].strict;
  const b = newScope(a, SC_BLOCK, s, func, st, strict);
  walkStmts(a, b, st.block);
  const h = st.handler;
  if (h !== null) {
    let outer = s;
    if (h.param !== null) {
      const c = newScope(a, SC_CATCH, s, func, h, strict);
      const names = [];
      patNames(h.param, names);
      let i = 0;
      while (i < names.length) {
        const id = names[i];
        const bid = newBinding(a, c, id.name, B_CATCH, id, 0, false);
        noteDecl(a, id, bid);
        i = i + 1;
      }
      walkBindingPat(a, c, h.param, false);
      outer = c;
    }
    const body = newScope(a, SC_BLOCK, outer, func, h, strict);
    walkStmts(a, body, h.body);
  }
  if (st.finalizer !== null) {
    const fin = newScope(a, SC_BLOCK, s, func, st.finalizer, strict);
    walkStmts(a, fin, st.finalizer);
  }
}

function walkImport(a, s, d) {
  let i = 0;
  while (i < d.specifiers.length) {
    const sp = d.specifiers[i];
    let id = sp.ident;
    if (sp.kind === IS_NAMED) {
      id = sp.local;
    }
    // A named or default import reads the exporting module's binding,
    // which a cycle can leave uninitialized at any point (CreateImportBinding
    // and GetBindingValue of module environments, 9.1.1.5.1): every read is
    // checked. A namespace object is there from the start.
    let b = -1;
    if (sp.kind === IS_NAMESPACE) {
      b = newBinding(a, s, id.name, B_IMPORT, id, 0, false);
    } else {
      b = newBinding(a, s, id.name, B_IMPORT, id, ALWAYS_CHECK, true);
    }
    noteDecl(a, id, b);
    i = i + 1;
  }
}

function markExported(a, s, id) {
  const b = lookupHere(a, s, id.name);
  if (b >= 0) {
    a.bindings[b].exported = true;
  }
}

function walkExport(a, s, st) {
  const d = st.decl;
  const k = d.kind;
  if (k === ED_NAMED) {
    if (d.source === null) {
      let i = 0;
      while (i < d.specifiers.length) {
        const local = d.specifiers[i].local;
        if (local.kind === MN_IDENT) {
          addRef(a, s, local.ident.name, local.ident, R_EXPORT);
        }
        i = i + 1;
      }
    }
  } else if (k === ED_DECL) {
    const inner = d.stmt;
    walkStmt(a, s, inner);
    if (inner.kind === S_VAR) {
      let i = 0;
      while (i < inner.decl.decls.length) {
        const names = [];
        patNames(inner.decl.decls[i].target, names);
        let j = 0;
        while (j < names.length) {
          markExported(a, s, names[j]);
          j = j + 1;
        }
        i = i + 1;
      }
    } else if (inner.kind === S_FUNCTION) {
      markExported(a, s, inner.func.id);
    } else {
      markExported(a, s, inner.cls.id);
    }
  } else if (k === ED_DEFAULT) {
    const x = d.def;
    if (x.kind === EX_FUNCTION) {
      const fn = x.func;
      if (fn.id !== null) {
        a.bindings[declareFunctionDecl(a, s, fn, fn.id, false)].exported = true;
      } else {
        const b = newBinding(a, s, DEFAULT_NAME, B_FUNCTION, fn, 0, false);
        a.bindings[b].exported = true;
        a.scopes[s].funcInits.push(fn);
      }
      walkFunction(a, s, fn, FN_FUNCTION, false);
    } else if (x.kind === EX_CLASS) {
      const c = x.cls;
      if (c.id !== null) {
        walkClass(a, s, c, true);
        markExported(a, s, c.id);
      } else {
        const b = newBinding(a, s, DEFAULT_NAME, B_CLASS, c, c.end, true);
        a.bindings[b].exported = true;
        walkClass(a, s, c, false);
      }
    } else {
      walkExpr(a, s, x.expr);
      const b = newBinding(a, s, DEFAULT_NAME, B_LET, st, st.end, true);
      a.bindings[b].exported = true;
    }
  }
}

// ---- functions and classes ----

function paramsOf(a, f, fs, params) {
  const fr = a.funcs[f];
  let i = 0;
  while (i < params.length) {
    const p = params[i];
    if (p.kind !== PT_IDENT) {
      fr.simpleParams = false;
    }
    if (patHasExpression(p)) {
      fr.paramExpressions = true;
    }
    i = i + 1;
  }
  i = 0;
  while (i < params.length) {
    const names = [];
    patNames(params[i], names);
    let j = 0;
    while (j < names.length) {
      const id = names[j];
      let b = lookupHere(a, fs, id.name);
      if (b < 0) {
        // Without parameter expressions there is nothing to observe the
        // TDZ of a parameter.
        b = newBinding(a, fs, id.name, B_PARAM, id, params[i].end, fr.paramExpressions);
        nameMapSet(fr.paramNames, id.name, b);
      }
      noteDecl(a, id, b);
      j = j + 1;
    }
    i = i + 1;
  }
  i = 0;
  while (i < params.length) {
    walkBindingPat(a, fs, params[i], false);
    i = i + 1;
  }
}

// The scope for the body: the parameter scope, or a separate one when
// parameters have expressions (10.2.11 step 28).
function bodyScope(a, f, fs, node) {
  if (!a.funcs[f].paramExpressions) {
    return fs;
  }
  return newScope(a, SC_BODY, fs, f, node, a.funcs[f].strict);
}

function walkFunction(a, s, fn, kind, isExpr) {
  let outer = s;
  const parent = a.scopes[s].func;
  const f = newFunc(a, kind, fn, parent, fn.strict);
  if (isExpr && fn.id !== null) {
    // 15.2.5: a named function expression's name lives in a scope of its
    // own around the function.
    outer = newScope(a, SC_FUNC_NAME, s, parent, fn, fn.strict);
    const b = newBinding(a, outer, fn.id.name, B_FUNC_NAME, fn.id, 0, false);
    noteDecl(a, fn.id, b);
  }
  const fs = newScope(a, SC_FUNCTION, outer, f, fn, fn.strict);
  a.funcs[f].scope = fs;
  paramsOf(a, f, fs, fn.params);
  const vs = bodyScope(a, f, fs, fn);
  a.funcs[f].varScope = vs;
  walkStmts(a, vs, fn.body);
}

function walkArrow(a, s, ar) {
  const f = newFunc(a, FN_ARROW, ar, a.scopes[s].func, ar.strict);
  const fs = newScope(a, SC_FUNCTION, s, f, ar, ar.strict);
  a.funcs[f].scope = fs;
  paramsOf(a, f, fs, ar.params);
  const vs = bodyScope(a, f, fs, ar);
  a.funcs[f].varScope = vs;
  if (ar.body.kind === AB_EXPR) {
    walkExpr(a, vs, ar.body.expr);
  } else {
    walkStmts(a, vs, ar.body.body);
  }
}

// A function-like piece of a class body with no parameters: a field
// initializer or a static block.
function walkClassPiece(a, s, node, kind) {
  const f = newFunc(a, kind, node, a.scopes[s].func, true);
  const fs = newScope(a, SC_FUNCTION, s, f, node, true);
  a.funcs[f].scope = fs;
  a.funcs[f].varScope = fs;
  return fs;
}

function privateKindOf(m) {
  if (m.kind === CM_FIELD) {
    return PN_FIELD;
  }
  if (m.methodKind === MK_GET) {
    return PN_GET;
  }
  if (m.methodKind === MK_SET) {
    return PN_SET;
  }
  return PN_METHOD;
}

// A class (15.7.14 ClassDefinitionEvaluation). isDecl: a declaration,
// whose name is also bound in scope s.
function walkClass(a, s, c, isDecl) {
  const func = a.scopes[s].func;
  if (isDecl && c.id !== null) {
    const b = newBinding(a, s, c.id.name, B_CLASS, c.id, c.end, true);
    noteDecl(a, c.id, b);
  }
  const cs = newScope(a, SC_CLASS, s, func, c, true);
  if (c.id !== null) {
    // The inner name: initialised once the class is defined, before
    // static fields and blocks run.
    const inner = newBinding(a, cs, c.id.name, B_CLASS_NAME, c.id, c.end, true);
    if (!isDecl) {
      noteDecl(a, c.id, inner);
    }
  }
  if (c.superClass !== null) {
    walkExpr(a, cs, c.superClass);
  }
  const body = newScope(a, SC_CLASS_BODY, cs, func, c, true);
  const bs = a.scopes[body];
  bs.privates = newNameMap();
  let i = 0;
  while (i < c.body.length) {
    const m = c.body[i];
    if ((m.kind === CM_METHOD || m.kind === CM_FIELD) && m.key.kind === PK_PRIVATE) {
      const id = m.key.ident;
      const pk = privateKindOf(m);
      const old = nameMapGet(bs.privates, id.name);
      if (old === null) {
        const b = {
          id: a.bindings.length,
          name: id.name,
          kind: B_PRIVATE,
          scope: body,
          decl: id,
          declEnd: 0,
          tdz: false,
          captured: false,
          capturedBy: [],
          written: false,
          refCount: 0,
          mutatedAfterCapture: false,
          loopFresh: false,
          exported: false,
          annexVar: -1,
          fromParam: -1,
          privateKind: pk,
          isStatic: m.isStatic,
          annexOnly: false,
        };
        a.bindings.push(b);
        bs.bindings.push(b.id);
        nameMapSet(bs.privates, id.name, b.id);
        noteDecl(a, id, b.id);
      } else {
        a.bindings[old].privateKind = PN_ACCESSOR;
        noteDecl(a, id, old);
      }
    }
    i = i + 1;
  }
  i = 0;
  while (i < c.body.length) {
    const m = c.body[i];
    if (m.kind === CM_METHOD) {
      if (m.key.kind === PK_COMPUTED) {
        walkExpr(a, body, m.key.expr);
      }
      let kind = FN_METHOD;
      if (m.methodKind === MK_GET) {
        kind = FN_GETTER;
      } else if (m.methodKind === MK_SET) {
        kind = FN_SETTER;
      } else if (m.methodKind === MK_CONSTRUCTOR) {
        kind = FN_CONSTRUCTOR;
      }
      walkFunction(a, body, m.value, kind, false);
    } else if (m.kind === CM_FIELD) {
      if (m.key.kind === PK_COMPUTED) {
        walkExpr(a, body, m.key.expr);
      }
      if (m.value !== null) {
        const fs = walkClassPiece(a, body, m, FN_FIELD);
        walkExpr(a, fs, m.value);
      }
    } else {
      const fs = walkClassPiece(a, body, m, FN_STATIC_BLOCK);
      walkStmts(a, fs, m.body);
    }
    i = i + 1;
  }
}

// ---- expressions ----

function useThis(a, s) {
  const f = thisFunc(a, s);
  a.funcs[f].usesThis = true;
  if (a.scopes[s].func !== f) {
    a.funcs[f].thisFromArrows = true;
  }
  return f;
}

// A direct eval (19.2.1.1): it can see every binding in scope by name,
// use `this`, `arguments` and `new.target` of the function, and in sloppy
// code add `var` bindings to the var scope.
function markEval(a, s) {
  const own = a.scopes[s].func;
  a.funcs[own].directEval = true;
  if (!a.scopes[s].strict) {
    a.funcs[own].sloppyEval = true;
    a.scopes[a.funcs[own].varScope].grows = true;
    a.scopes[a.funcs[own].scope].grows = true;
  }
  let f = own;
  a.funcs[f].evalInside = true;
  while (a.funcs[f].arrow) {
    f = a.funcs[f].parent;
    a.funcs[f].evalInside = true;
  }
  useThis(a, s);
  const t = a.funcs[f];
  t.mentionsArguments = true;
  if (t.kind !== FN_SCRIPT && t.kind !== FN_MODULE) {
    t.usesNewTarget = true;
  }
  let c = s;
  while (c >= 0) {
    a.scopes[c].evalVisible = true;
    c = a.scopes[c].parent;
  }
}

function walkExprs(a, s, list) {
  let i = 0;
  while (i < list.length) {
    if (list[i] !== null) {
      walkExpr(a, s, list[i]);
    }
    i = i + 1;
  }
}

function walkExpr(a, s, e) {
  const k = e.kind;
  if (k === E_IDENT) {
    addRef(a, s, e.name, e, R_READ);
  } else if (k === E_THIS) {
    const f = useThis(a, s);
    if (a.scopes[s].func !== f) {
      a.funcs[f].thisWordFromArrows = true;
    }
  } else if (k === E_TEMPLATE) {
    walkExprs(a, s, e.quasi.exprs);
  } else if (k === E_TAGGED) {
    walkExpr(a, s, e.tag);
    walkExprs(a, s, e.quasi.exprs);
  } else if (k === E_ARRAY) {
    walkExprs(a, s, e.items);
  } else if (k === E_OBJECT) {
    let i = 0;
    while (i < e.props.length) {
      const p = e.props[i];
      if (p.kind === PR_INIT) {
        if (p.key.kind === PK_COMPUTED) {
          walkExpr(a, s, p.key.expr);
        }
        walkExpr(a, s, p.value);
      } else if (p.kind === PR_SHORTHAND) {
        addRef(a, s, p.ident.name, p.ident, R_READ);
      } else if (p.kind === PR_METHOD) {
        if (p.key.kind === PK_COMPUTED) {
          walkExpr(a, s, p.key.expr);
        }
        let kind = FN_METHOD;
        if (p.methodKind === MK_GET) {
          kind = FN_GETTER;
        } else if (p.methodKind === MK_SET) {
          kind = FN_SETTER;
        }
        walkFunction(a, s, p.value, kind, false);
      } else {
        walkExpr(a, s, p.arg);
      }
      i = i + 1;
    }
  } else if (k === E_FUNCTION) {
    walkFunction(a, s, e.func, FN_FUNCTION, true);
  } else if (k === E_ARROW) {
    walkArrow(a, s, e.arrow);
  } else if (k === E_CLASS) {
    walkClass(a, s, e.cls, false);
  } else if (k === E_UNARY) {
    const x = stripParens(e.arg);
    if ((e.op === U_DELETE || e.op === U_TYPEOF) && x.kind === E_IDENT) {
      const r = addRef(a, s, x.name, x, R_READ);
      if (e.op === U_DELETE) {
        r.deleteRef = true;
      } else {
        r.typeofRef = true;
      }
    } else {
      walkExpr(a, s, e.arg);
    }
  } else if (k === E_UPDATE) {
    walkExprTarget(a, s, e.arg, R_READ_WRITE);
  } else if (k === E_BINARY || k === E_LOGICAL) {
    walkExpr(a, s, e.left);
    walkExpr(a, s, e.right);
  } else if (k === E_ASSIGN) {
    let kind = R_READ_WRITE;
    if (e.op === A_ASSIGN) {
      kind = R_WRITE;
    }
    walkTarget(a, s, e.target, kind);
    walkExpr(a, s, e.value);
  } else if (k === E_COND) {
    walkExpr(a, s, e.test);
    walkExpr(a, s, e.cons);
    walkExpr(a, s, e.alt);
  } else if (k === E_CALL) {
    if (e.callee.kind === E_SUPER) {
      const f = useThis(a, s);
      a.funcs[f].usesSuperCall = true;
    } else {
      walkExpr(a, s, e.callee);
      const x = stripParens(e.callee);
      if (!e.optional && x.kind === E_IDENT && sameUnits(x.name, EVAL)) {
        markEval(a, s);
      }
    }
    walkExprs(a, s, e.args);
  } else if (k === E_NEW) {
    walkExpr(a, s, e.callee);
    walkExprs(a, s, e.args);
  } else if (k === E_MEMBER) {
    if (e.object.kind === E_SUPER) {
      const f = useThis(a, s);
      a.funcs[f].usesSuperProperty = true;
    } else {
      walkExpr(a, s, e.object);
    }
    const p = e.prop;
    if (p.kind === MP_COMPUTED) {
      walkExpr(a, s, p.expr);
    } else if (p.kind === MP_PRIVATE) {
      addRef(a, s, p.ident.name, p.ident, R_PRIVATE);
    }
  } else if (k === E_OPT_CHAIN || k === E_PAREN) {
    walkExpr(a, s, e.expr);
  } else if (k === E_SEQ) {
    walkExprs(a, s, e.items);
  } else if (k === E_SPREAD || k === E_AWAIT) {
    walkExpr(a, s, e.arg);
  } else if (k === E_YIELD) {
    if (e.arg !== null) {
      walkExpr(a, s, e.arg);
    }
  } else if (k === E_NEW_TARGET) {
    a.funcs[thisFunc(a, s)].usesNewTarget = true;
  } else if (k === E_IMPORT) {
    walkExpr(a, s, e.source);
    if (e.options !== null) {
      walkExpr(a, s, e.options);
    }
  } else if (k === E_PRIVATE_IN) {
    addRef(a, s, e.name.name, e.name, R_PRIVATE);
    walkExpr(a, s, e.right);
  }
  // E_SUPER only appears as a callee or member object; E_IMPORT_META and
  // literals bind nothing.
}

// ---- decisions after the walk ----

function isLexicalKind(k) {
  return k === B_LET || k === B_CONST || k === B_CLASS || k === B_FUNCTION || k === B_IMPORT || k === B_CLASS_NAME;
}

// Annex B.3.2.1 and B.3.2.2: a plain function declared in a block of
// sloppy code is also a `var` of the enclosing function (or script) when
// a `var` of that name there would not be an early error and the name is
// not a parameter. Its value is copied to that `var` when the declaration
// is evaluated.
function decideAnnex(a) {
  let i = 0;
  while (i < a.annexCandidates.length) {
    const fb = a.bindings[a.annexCandidates[i]];
    const blk = fb.scope;
    const f = a.funcs[a.scopes[blk].func];
    let ok = nameMapGet(f.paramNames, fb.name) === null;
    let c = a.scopes[blk].parent;
    while (ok && c >= 0) {
      const sc = a.scopes[c];
      const b = lookupHere(a, c, fb.name);
      if (b >= 0) {
        const k = a.bindings[b].kind;
        if (c === f.varScope) {
          // Var-scoped functions and vars are fine; a lexical name of the
          // body (or of the script) is not.
          if (k === B_LET || k === B_CONST || k === B_CLASS || k === B_IMPORT) {
            ok = false;
          }
        } else if (k === B_CATCH) {
          // B.3.4: a `var` may reuse a catch parameter that is a plain name.
          const h = sc.node;
          ok = h.param !== null && h.param.kind === PT_IDENT;
        } else if (isLexicalKind(k)) {
          ok = false;
        }
      }
      if (c === f.varScope) {
        break;
      }
      c = sc.parent;
    }
    if (ok) {
      const vs = f.varScope;
      let vb = lookupHere(a, vs, fb.name);
      if (vb < 0) {
        vb = newBinding(a, vs, fb.name, B_VAR, fb.decl, 0, false);
        a.bindings[vb].annexOnly = true;
      }
      fb.annexVar = vb;
    }
    i = i + 1;
  }
}

// FunctionDeclarationInstantiation steps 15 to 18: whether a function has
// its own `arguments` object. It is created only when something can use
// it: a reference to the name, or a direct eval.
function decideArguments(a) {
  let i = 0;
  while (i < a.funcs.length) {
    const f = a.funcs[i];
    const k = f.kind;
    const own = k === FN_FUNCTION || k === FN_METHOD || k === FN_GETTER || k === FN_SETTER || k === FN_CONSTRUCTOR;
    if (own && f.mentionsArguments && nameMapGet(f.paramNames, ARGUMENTS) === null) {
      let needed = true;
      if (!f.paramExpressions) {
        const b = lookupHere(a, f.varScope, ARGUMENTS);
        if (b >= 0) {
          const bk = a.bindings[b].kind;
          if (bk === B_FUNCTION || bk === B_LET || bk === B_CONST || bk === B_CLASS) {
            needed = false;
          }
        }
      }
      if (needed) {
        // A `var arguments` next to the parameters is the same binding,
        // and starts as the arguments object (FunctionDeclarationInstantiation
        // binds it with the parameters, and a var of a parameter name is not
        // made again). A body `var arguments` behind parameter expressions
        // starts with its value.
        const same = lookupHere(a, f.scope, ARGUMENTS);
        if (same >= 0) {
          f.argumentsBinding = same;
        } else {
          f.argumentsBinding = newBinding(a, f.scope, ARGUMENTS, B_ARGUMENTS, f.node, 0, false);
          if (f.varScope !== f.scope) {
            const copy = lookupHere(a, f.varScope, ARGUMENTS);
            if (copy >= 0 && a.bindings[copy].kind === B_VAR) {
              a.bindings[copy].fromParam = f.argumentsBinding;
            }
          }
        }
        a.bindings[f.argumentsBinding].argumentsObject = true;
        f.mapped = !f.strict && f.simpleParams;
      }
    }
    i = i + 1;
  }
}

// The enclosing class body declaring private name `name`, from scope s.
function resolvePrivate(a, r) {
  let c = r.scope;
  while (c >= 0) {
    const sc = a.scopes[c];
    if (sc.kind === SC_CLASS_BODY) {
      const b = nameMapGet(sc.privates, r.name);
      if (b !== null) {
        return b;
      }
    }
    c = sc.parent;
  }
  return -1;
}

function noteCapture(a, b, fromFunc) {
  const bind = a.bindings[b];
  const home = a.scopes[bind.scope].func;
  let f = fromFunc;
  while (f >= 0 && f !== home) {
    const caps = a.funcs[f].captures;
    let seen = false;
    let i = 0;
    while (i < caps.length) {
      if (caps[i] === b) {
        seen = true;
        break;
      }
      i = i + 1;
    }
    if (seen) {
      break;
    }
    caps.push(b);
    f = a.funcs[f].parent;
  }
  bind.captured = true;
  let j = 0;
  while (j < bind.capturedBy.length) {
    if (bind.capturedBy[j] === fromFunc) {
      return;
    }
    j = j + 1;
  }
  bind.capturedBy.push(fromFunc);
}

function resolveRef(a, r) {
  let b = -1;
  if (r.kind === R_PRIVATE) {
    b = resolvePrivate(a, r);
  } else {
    let c = r.scope;
    while (c >= 0) {
      const sc = a.scopes[c];
      if (sc.kind === SC_WITH) {
        r.dynamic = true;
      } else {
        b = lookupHere(a, c, r.name);
        if (b >= 0) {
          break;
        }
        if (sc.grows) {
          r.dynamic = true;
        }
      }
      c = sc.parent;
    }
  }
  if (b < 0) {
    r.resolution = RES_GLOBAL;
    return;
  }
  const bind = a.bindings[b];
  r.binding = b;
  bind.refCount = bind.refCount + 1;
  if (r.kind === R_WRITE || r.kind === R_READ_WRITE) {
    bind.written = true;
  } else if (r.kind === R_EXPORT) {
    bind.exported = true;
  }
  const home = a.scopes[bind.scope].func;
  if (home === r.func) {
    r.resolution = RES_LOCAL;
  } else {
    r.resolution = RES_CAPTURED;
    noteCapture(a, b, r.func);
  }
  if (bind.tdz) {
    r.tdzCheck = home !== r.func || r.node.start < bind.declEnd || a.scopes[bind.scope].kind === SC_SWITCH;
  }
}

function resolveAll(a) {
  let i = 0;
  while (i < a.refs.length) {
    resolveRef(a, a.refs[i]);
    i = i + 1;
  }
}

function finish(a) {
  let i = 0;
  while (i < a.bindings.length) {
    const b = a.bindings[i];
    b.mutatedAfterCapture = b.captured && b.written;
    const sc = a.scopes[b.scope];
    if (sc.kind === SC_FOR && b.captured) {
      // 14.7.4.4 CreatePerIterationEnvironment copies `let` bindings of
      // a `for (;;)` head; for-in and for-of make new bindings for every
      // iteration (14.7.5.7). Only a closure can tell the difference.
      b.loopFresh = sc.node.kind !== S_FOR || b.kind === B_LET;
    }
    i = i + 1;
  }
  i = 0;
  while (i < a.funcs.length) {
    const f = a.funcs[i];
    if (f.argumentsBinding >= 0) {
      f.usesArguments = a.bindings[f.argumentsBinding].refCount > 0 || f.evalInside;
    }
    i = i + 1;
  }
}

// The reference recorded for identifier node n (-1: none). Linear: for
// tests and checks.
export function refAt(a, n) {
  let i = 0;
  while (i < a.refs.length) {
    if (a.refs[i].node === n) {
      return i;
    }
    i = i + 1;
  }
  return -1;
}
