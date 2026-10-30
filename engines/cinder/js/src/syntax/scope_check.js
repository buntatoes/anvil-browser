// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// A consistency check of a scope analysis (src/syntax/scope.js) against
// the syntax tree it came from. It walks the tree on its own, listing
// every identifier that declares a binding and every identifier that is
// a reference, and then checks that:
//
// * each declaring identifier is recorded once as a declaration (and a
//   `var` with an initializer or in a for-in/of head once as a write too);
// * each reference identifier has exactly one reference record, and the
//   analysis records nothing for nodes the walk did not list;
// * every binding has a declaration site, sits in its scope's list and
//   name table, and no scope has two bindings of one name;
// * each resolved reference names its binding, the binding's scope
//   encloses the reference, and no nearer scope declares the name;
// * local/captured agree with the functions involved, captured bindings
//   appear in the capture lists of every function in between, every
//   capture list entry is backed by a reference, and reference counts
//   add up;
// * the scope tree and the function tree are consistent.
//
// checkScopes(prog, a) returns a list of problem strings (empty: fine).

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
  PT_EXPR,
  PP_PROP,
  IS_NAMED,
  ED_NAMED,
  ED_DECL,
  ED_DEFAULT,
  EX_FUNCTION,
  EX_CLASS,
  MN_IDENT,
  VK_VAR,
} from "./ast.js";
import { sameUnits, nameMapGet } from "./names.js";
import { SC_FUNCTION, SC_WITH, SC_CLASS_BODY, B_PRIVATE, R_PRIVATE, RES_LOCAL, RES_CAPTURED } from "./scope.js";

const ARGUMENTS_NAME = [0x61, 0x72, 0x67, 0x75, 0x6d, 0x65, 0x6e, 0x74, 0x73]; // "arguments"

// What the walk expects at an identifier node.
const WANT_DECL = 1;
const WANT_DECL_AND_INIT = 2;
const WANT_REF = 3;

function newCensus() {
  return { want: new Map(), nodes: [], count: 0 };
}

function expect(c, node, what) {
  if (!c.want.has(node)) {
    c.nodes.push(node);
  }
  c.want.set(node, what);
  c.count += 1;
}

function patDecl(c, p, init) {
  const k = p.kind;
  if (k === PT_IDENT) {
    expect(c, p, init ? WANT_DECL_AND_INIT : WANT_DECL);
  } else if (k === PT_ARRAY) {
    for (const x of p.items) {
      if (x !== null) {
        patDecl(c, x, init);
      }
    }
  } else if (k === PT_OBJECT) {
    for (const pp of p.props) {
      if (pp.kind === PP_PROP) {
        if (pp.key.kind === PK_COMPUTED) {
          expr(c, pp.key.expr);
        }
        patDecl(c, pp.value, init);
      } else {
        patDecl(c, pp.arg, init);
      }
    }
  } else if (k === PT_ASSIGN) {
    patDecl(c, p.target, init);
    expr(c, p.def);
  } else if (k === PT_REST) {
    patDecl(c, p.arg, init);
  }
}

function patTarget(c, p) {
  const k = p.kind;
  if (k === PT_IDENT) {
    expect(c, p, WANT_REF);
  } else if (k === PT_ARRAY) {
    for (const x of p.items) {
      if (x !== null) {
        patTarget(c, x);
      }
    }
  } else if (k === PT_OBJECT) {
    for (const pp of p.props) {
      if (pp.kind === PP_PROP) {
        if (pp.key.kind === PK_COMPUTED) {
          expr(c, pp.key.expr);
        }
        patTarget(c, pp.value);
      } else {
        patTarget(c, pp.arg);
      }
    }
  } else if (k === PT_ASSIGN) {
    patTarget(c, p.target);
    expr(c, p.def);
  } else if (k === PT_REST) {
    patTarget(c, p.arg);
  } else if (k === PT_EXPR) {
    expr(c, p.expr);
  }
}

function varDecl(c, d) {
  for (const x of d.decls) {
    if (x.init !== null) {
      expr(c, x.init);
    }
    patDecl(c, x.target, d.varKind === VK_VAR && x.init !== null);
  }
}

function func(c, f, withName) {
  if (withName && f.id !== null) {
    expect(c, f.id, WANT_DECL);
  }
  for (const p of f.params) {
    patDecl(c, p, false);
  }
  stmts(c, f.body);
}

function cls(c, k) {
  if (k.id !== null) {
    expect(c, k.id, WANT_DECL);
  }
  if (k.superClass !== null) {
    expr(c, k.superClass);
  }
  for (const m of k.body) {
    if (m.kind === CM_METHOD || m.kind === CM_FIELD) {
      if (m.key.kind === PK_COMPUTED) {
        expr(c, m.key.expr);
      } else if (m.key.kind === PK_PRIVATE) {
        expect(c, m.key.ident, WANT_DECL);
      }
      if (m.kind === CM_METHOD) {
        func(c, m.value, false);
      } else if (m.value !== null) {
        expr(c, m.value);
      }
    } else {
      stmts(c, m.body);
    }
  }
}

function stmts(c, list) {
  for (const s of list) {
    stmt(c, s);
  }
}

function stmt(c, s) {
  const k = s.kind;
  if (k === S_EXPR) {
    expr(c, s.expr);
  } else if (k === S_THROW) {
    expr(c, s.arg);
  } else if (k === S_VAR) {
    varDecl(c, s.decl);
  } else if (k === S_FUNCTION) {
    func(c, s.func, true);
  } else if (k === S_CLASS) {
    cls(c, s.cls);
  } else if (k === S_BLOCK) {
    stmts(c, s.body);
  } else if (k === S_IF) {
    expr(c, s.test);
    stmt(c, s.cons);
    if (s.alt !== null) {
      stmt(c, s.alt);
    }
  } else if (k === S_FOR) {
    if (s.init !== null) {
      if (s.init.kind === FI_VAR) {
        varDecl(c, s.init.decl);
      } else {
        expr(c, s.init.expr);
      }
    }
    if (s.test !== null) {
      expr(c, s.test);
    }
    if (s.update !== null) {
      expr(c, s.update);
    }
    stmt(c, s.body);
  } else if (k === S_FOR_IN || k === S_FOR_OF) {
    if (s.left.kind === FH_VAR) {
      const d = s.left.decl;
      for (const x of d.decls) {
        if (x.init !== null) {
          expr(c, x.init);
        }
        patDecl(c, x.target, d.varKind === VK_VAR);
      }
    } else {
      patTarget(c, s.left.pat);
    }
    expr(c, s.right);
    stmt(c, s.body);
  } else if (k === S_WHILE || k === S_DO_WHILE) {
    expr(c, s.test);
    stmt(c, s.body);
  } else if (k === S_RETURN) {
    if (s.arg !== null) {
      expr(c, s.arg);
    }
  } else if (k === S_WITH) {
    expr(c, s.object);
    stmt(c, s.body);
  } else if (k === S_SWITCH) {
    expr(c, s.disc);
    for (const cs of s.cases) {
      if (cs.test !== null) {
        expr(c, cs.test);
      }
      stmts(c, cs.body);
    }
  } else if (k === S_LABELED) {
    stmt(c, s.body);
  } else if (k === S_TRY) {
    stmts(c, s.block);
    if (s.handler !== null) {
      if (s.handler.param !== null) {
        patDecl(c, s.handler.param, false);
      }
      stmts(c, s.handler.body);
    }
    if (s.finalizer !== null) {
      stmts(c, s.finalizer);
    }
  } else if (k === S_IMPORT) {
    for (const sp of s.decl.specifiers) {
      if (sp.kind === IS_NAMED) {
        expect(c, sp.local, WANT_DECL);
      } else {
        expect(c, sp.ident, WANT_DECL);
      }
    }
  } else if (k === S_EXPORT) {
    const d = s.decl;
    if (d.kind === ED_NAMED) {
      if (d.source === null) {
        for (const sp of d.specifiers) {
          if (sp.local.kind === MN_IDENT) {
            expect(c, sp.local.ident, WANT_REF);
          }
        }
      }
    } else if (d.kind === ED_DECL) {
      stmt(c, d.stmt);
    } else if (d.kind === ED_DEFAULT) {
      const x = d.def;
      if (x.kind === EX_FUNCTION) {
        func(c, x.func, true);
      } else if (x.kind === EX_CLASS) {
        cls(c, x.cls);
      } else {
        expr(c, x.expr);
      }
    }
  }
}

function exprs(c, list) {
  for (const e of list) {
    if (e !== null) {
      expr(c, e);
    }
  }
}

function expr(c, e) {
  const k = e.kind;
  if (k === E_IDENT) {
    expect(c, e, WANT_REF);
  } else if (k === E_TEMPLATE) {
    exprs(c, e.quasi.exprs);
  } else if (k === E_TAGGED) {
    expr(c, e.tag);
    exprs(c, e.quasi.exprs);
  } else if (k === E_ARRAY) {
    exprs(c, e.items);
  } else if (k === E_OBJECT) {
    for (const p of e.props) {
      if (p.kind === PR_INIT) {
        if (p.key.kind === PK_COMPUTED) {
          expr(c, p.key.expr);
        }
        expr(c, p.value);
      } else if (p.kind === PR_SHORTHAND) {
        expect(c, p.ident, WANT_REF);
      } else if (p.kind === PR_METHOD) {
        if (p.key.kind === PK_COMPUTED) {
          expr(c, p.key.expr);
        }
        func(c, p.value, false);
      } else {
        expr(c, p.arg);
      }
    }
  } else if (k === E_FUNCTION) {
    func(c, e.func, true);
  } else if (k === E_ARROW) {
    for (const p of e.arrow.params) {
      patDecl(c, p, false);
    }
    if (e.arrow.body.kind === AB_EXPR) {
      expr(c, e.arrow.body.expr);
    } else {
      stmts(c, e.arrow.body.body);
    }
  } else if (k === E_CLASS) {
    cls(c, e.cls);
  } else if (k === E_UNARY || k === E_UPDATE || k === E_SPREAD || k === E_AWAIT) {
    expr(c, e.arg);
  } else if (k === E_BINARY || k === E_LOGICAL) {
    expr(c, e.left);
    expr(c, e.right);
  } else if (k === E_ASSIGN) {
    patTarget(c, e.target);
    expr(c, e.value);
  } else if (k === E_COND) {
    expr(c, e.test);
    expr(c, e.cons);
    expr(c, e.alt);
  } else if (k === E_CALL || k === E_NEW) {
    expr(c, e.callee);
    exprs(c, e.args);
  } else if (k === E_MEMBER) {
    expr(c, e.object);
    if (e.prop.kind === MP_COMPUTED) {
      expr(c, e.prop.expr);
    } else if (e.prop.kind === MP_PRIVATE) {
      expect(c, e.prop.ident, WANT_REF);
    }
  } else if (k === E_OPT_CHAIN || k === E_PAREN) {
    expr(c, e.expr);
  } else if (k === E_SEQ) {
    exprs(c, e.items);
  } else if (k === E_YIELD) {
    if (e.arg !== null) {
      expr(c, e.arg);
    }
  } else if (k === E_IMPORT) {
    expr(c, e.source);
    if (e.options !== null) {
      expr(c, e.options);
    }
  } else if (k === E_PRIVATE_IN) {
    expect(c, e.name, WANT_REF);
    expr(c, e.right);
  }
}

function contains(list, x) {
  for (const y of list) {
    if (y === x) {
      return true;
    }
  }
  return false;
}

function isAncestor(a, anc, s) {
  let c = s;
  while (c >= 0) {
    if (c === anc) {
      return true;
    }
    c = a.scopes[c].parent;
  }
  return false;
}

function nameText(units) {
  let s = "";
  for (const u of units) {
    s += String.fromCharCode(u);
  }
  return s;
}

function where(n) {
  return `${n.line}:${n.col}`;
}

// Check analysis a of program prog. Returns a list of problems.
export function checkScopes(prog, a) {
  const problems = [];
  function bad(text) {
    if (problems.length < 20) {
      problems.push(text);
    }
  }
  const c = newCensus();
  stmts(c, prog.body);
  if (c.want.size !== c.count) {
    bad(`the walk met ${c.count} identifier places but only ${c.want.size} distinct nodes`);
  }
  // Declarations and references recorded per node.
  const decls = new Map();
  for (let i = 0; i < a.declNodes.length; i++) {
    const n = a.declNodes[i];
    decls.set(n, (decls.get(n) || 0) + 1);
    if (!c.want.has(n)) {
      bad(`declaration recorded at ${where(n)}, which the walk did not list`);
    }
  }
  const refs = new Map();
  for (const r of a.refs) {
    refs.set(r.node, (refs.get(r.node) || 0) + 1);
    if (!c.want.has(r.node)) {
      bad(`reference recorded at ${where(r.node)}, which the walk did not list`);
    }
  }
  for (const n of c.nodes) {
    const w = c.want.get(n);
    const d = decls.get(n) || 0;
    const r = refs.get(n) || 0;
    const okDecl = w === WANT_REF ? d === 0 : d === 1;
    const okRef = w === WANT_DECL_AND_INIT ? r === 1 : w === WANT_REF ? r === 1 : r === 0;
    if (!okDecl || !okRef) {
      bad(`identifier ${nameText(n.name)} at ${where(n)}: ${d} declaration(s), ${r} reference(s)`);
    }
  }
  // Bindings.
  for (const b of a.bindings) {
    if (b.decl === null || b.decl === undefined) {
      bad(`binding #${b.id} ${nameText(b.name)} has no declaration site`);
    }
    const sc = a.scopes[b.scope];
    if (!contains(sc.bindings, b.id)) {
      bad(`binding #${b.id} is not listed in scope ${b.scope}`);
    }
    if (b.kind === B_PRIVATE) {
      if (sc.kind !== SC_CLASS_BODY || nameMapGet(sc.privates, b.name) !== b.id) {
        bad(`private name #${b.id} is not in its class body's table`);
      }
    } else if (nameMapGet(sc.names, b.name) !== b.id) {
      bad(`binding #${b.id} ${nameText(b.name)} is not the one its scope's table names`);
    }
  }
  // Body copies of parameters, and arguments objects.
  for (const b of a.bindings) {
    if (b.fromParam >= 0) {
      const p = a.bindings[b.fromParam];
      const f = a.funcs[a.scopes[b.scope].func];
      if (!sameUnits(p.name, b.name) || p.scope !== f.scope || b.scope !== f.varScope || f.scope === f.varScope) {
        bad(`binding #${b.id} copies #${p.id}, which is not its parameter-scope twin`);
      }
    }
  }
  for (const f of a.funcs) {
    if (f.argumentsBinding >= 0) {
      const b = a.bindings[f.argumentsBinding];
      if (!b.argumentsObject || b.scope !== f.scope || !sameUnits(b.name, ARGUMENTS_NAME)) {
        bad(`function #${f.id} has a wrong arguments binding #${b.id}`);
      }
    }
  }
  for (const sc of a.scopes) {
    for (let i = 0; i < sc.bindings.length; i++) {
      for (let j = i + 1; j < sc.bindings.length; j++) {
        const x = a.bindings[sc.bindings[i]];
        const y = a.bindings[sc.bindings[j]];
        if ((x.kind === B_PRIVATE) === (y.kind === B_PRIVATE) && sameUnits(x.name, y.name)) {
          bad(`scope ${sc.id} binds ${nameText(x.name)} twice`);
        }
      }
    }
    if (sc.parent >= 0) {
      const p = a.scopes[sc.parent];
      if (!contains(p.children, sc.id)) {
        bad(`scope ${sc.id} is not a child of its parent`);
      }
      if (sc.kind !== SC_FUNCTION && sc.func !== p.func) {
        bad(`scope ${sc.id} belongs to another function than its parent`);
      }
    }
    if (sc.kind === SC_FUNCTION && a.funcs[sc.func].scope !== sc.id) {
      bad(`function scope ${sc.id} is not its function's scope`);
    }
  }
  // References.
  const counts = new Map();
  const backed = new Map();
  for (const r of a.refs) {
    if (r.binding < 0) {
      continue;
    }
    const b = a.bindings[r.binding];
    counts.set(b.id, (counts.get(b.id) || 0) + 1);
    if (!sameUnits(b.name, r.name)) {
      bad(`reference at ${where(r.node)} resolves to a binding of another name`);
    }
    if ((r.kind === R_PRIVATE) !== (b.kind === B_PRIVATE)) {
      bad(`reference at ${where(r.node)} mixes private and plain names`);
    }
    if (!isAncestor(a, b.scope, r.scope)) {
      bad(`reference at ${where(r.node)} resolves to a binding out of scope`);
    }
    let s = r.scope;
    let throughWith = false;
    while (s !== b.scope) {
      const sc = a.scopes[s];
      if (sc.kind === SC_WITH) {
        throughWith = true;
      }
      if (r.kind === R_PRIVATE) {
        if (sc.privates !== null && nameMapGet(sc.privates, r.name) !== null) {
          bad(`reference at ${where(r.node)} skips a nearer private name`);
        }
      } else if (nameMapGet(sc.names, r.name) !== null) {
        bad(`reference at ${where(r.node)} skips a nearer binding`);
      }
      s = sc.parent;
    }
    if (throughWith && !r.dynamic) {
      bad(`reference at ${where(r.node)} passes a with scope but is not dynamic`);
    }
    const home = a.scopes[b.scope].func;
    if ((r.resolution === RES_LOCAL) !== (home === r.func)) {
      bad(`reference at ${where(r.node)}: local/captured does not match the functions`);
    }
    if (r.resolution === RES_CAPTURED) {
      if (!b.captured || !contains(b.capturedBy, r.func)) {
        bad(`reference at ${where(r.node)} is captured but its binding does not say so`);
      }
      let f = r.func;
      while (f >= 0 && f !== home) {
        if (!contains(a.funcs[f].captures, b.id)) {
          bad(`function #${f} lacks capture #${b.id} used at ${where(r.node)}`);
        }
        if (!backed.has(f)) {
          backed.set(f, new Set());
        }
        backed.get(f).add(b.id);
        f = a.funcs[f].parent;
      }
      if (f !== home) {
        bad(`reference at ${where(r.node)}: the binding's function does not enclose it`);
      }
    }
  }
  for (const b of a.bindings) {
    if ((counts.get(b.id) || 0) !== b.refCount) {
      bad(`binding #${b.id} counts ${b.refCount} references, ${counts.get(b.id) || 0} found`);
    }
    if (b.captured && b.capturedBy.length === 0) {
      bad(`binding #${b.id} is captured by no function`);
    }
  }
  for (const f of a.funcs) {
    const set = backed.get(f.id) || new Set();
    for (const id of f.captures) {
      if (!set.has(id)) {
        bad(`function #${f.id} captures #${id} with no reference behind it`);
      }
    }
    if (f.id > 0) {
      let p = f.parent;
      let steps = 0;
      while (p > 0 && steps <= a.funcs.length) {
        p = a.funcs[p].parent;
        steps += 1;
      }
      if (p !== 0) {
        bad(`function #${f.id} does not lead back to the program`);
      }
    }
  }
  return problems;
}

// The number of identifier places the walk lists (for reports).
export function identifierPlaces(prog) {
  const c = newCensus();
  stmts(c, prog.body);
  return c.count;
}
