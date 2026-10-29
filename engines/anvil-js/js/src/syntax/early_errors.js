// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// The early errors that need the declarations of a scope, a class body or
// a module (spec 8.2 and the Early Errors sections of clauses 14 to 16,
// with annex B.3.2 to B.3.4 for sloppy code):
//
// * a name declared twice where that is not allowed: lexical names
//   against any other declaration of the same scope, `var` names against
//   lexical names of every scope up to their function, parameters and
//   catch parameters against lexical names of their body;
// * private names declared twice in a class body (a getter and a setter
//   of the same placement may share one), and private names used where
//   no enclosing class declares them;
// * a module exporting one name twice, or exporting a local name it does
//   not declare.
//
// The parser calls these functions as it reads, at the places where the
// reference front end checks the same rules, so that a source with
// several errors reports the same one first. Each check returns false
// (or the offending node) and leaves the message to the parser.
//
// Scopes here are only what the rules need: a function scope (scripts,
// modules, functions, arrows, class field initializers and static
// blocks) or a block scope (blocks, `for`, `switch`, `catch` and the
// block annex B gives a function declared as an `if` branch).

import { newNameSet, nameSetHas, nameSetAdd, newNameMap, nameMapGet, nameMapSet } from "./names.js";

// Kinds of private names. A getter and a setter combine into GETSET.
export const PRIV_FIELD = 0;
export const PRIV_METHOD = 1;
export const PRIV_GET = 2;
export const PRIV_SET = 3;
export const PRIV_GETSET = 4;

export function newEarlyErrors(isModule) {
  return {
    isModule: isModule,
    scopes: [],
    // One record per enclosing class body: declared private names and
    // the references still to be resolved when it closes.
    classes: [],
    exported: newNameSet(),
    // `export { x }` without `from`: names checked at the end.
    exportLocals: [],
  };
}

function newScope(isFunction) {
  return {
    isFunction: isFunction,
    lexical: null,
    vars: null,
    annex: null,
    params: null,
    catchParams: null,
    catchSimple: false,
  };
}

function has(set, name) {
  return set !== null && nameSetHas(set, name);
}

export function enterScope(ee, isFunction) {
  ee.scopes.push(newScope(isFunction));
}

export function exitScope(ee) {
  ee.scopes.pop();
}

function top(ee) {
  return ee.scopes[ee.scopes.length - 1];
}

// The catch parameter names of the block just entered (a name set), and
// whether the parameter is a plain name (annex B.3.4 lets `var` reuse
// it then).
export function setCatchParams(ee, names, simple) {
  const s = top(ee);
  s.catchParams = names;
  s.catchSimple = simple;
}

// The parameter names of the function scope just entered.
export function setParams(ee, idents) {
  const s = top(ee);
  if (s.params === null) {
    s.params = newNameSet();
  }
  let i = 0;
  while (i < idents.length) {
    nameSetAdd(s.params, idents[i].name);
    i = i + 1;
  }
}

// A `let`, `const`, class, import or block-level function name.
export function declareLexical(ee, name) {
  const s = top(ee);
  if (has(s.lexical, name) || has(s.vars, name) || has(s.params, name) || has(s.catchParams, name)) {
    return false;
  }
  if (s.lexical === null) {
    s.lexical = newNameSet();
  }
  nameSetAdd(s.lexical, name);
  return true;
}

// A `var` name (or a function declared at the top of a function body):
// noted in every scope up to the function, where it lives.
export function declareVar(ee, name) {
  let i = ee.scopes.length - 1;
  while (i >= 0) {
    const s = ee.scopes[i];
    if (has(s.lexical, name) || (has(s.catchParams, name) && !s.catchSimple)) {
      return false;
    }
    if (s.vars === null) {
      s.vars = newNameSet();
    }
    nameSetAdd(s.vars, name);
    if (s.isFunction) {
      return true;
    }
    i = i - 1;
  }
  return true;
}

// A function declaration's name. At the top of a function or script it
// is a `var` name; at the top of a module and in blocks it is lexical,
// except that in sloppy blocks plain functions may share a name with
// each other (annex B.3.2.4 and B.3.2.5). `plain`: not async and not a
// generator.
export function declareFunction(ee, name, plain, strict) {
  const s = top(ee);
  const moduleTop = ee.isModule && ee.scopes.length === 1;
  if (s.isFunction && !moduleTop) {
    return declareVar(ee, name);
  }
  if (!s.isFunction && !strict && plain) {
    if (has(s.lexical, name) && has(s.annex, name)) {
      return true;
    }
    if (!declareLexical(ee, name)) {
      return false;
    }
    if (s.annex === null) {
      s.annex = newNameSet();
    }
    nameSetAdd(s.annex, name);
    return true;
  }
  return declareLexical(ee, name);
}

// ---- private names ----

export function openClass(ee) {
  ee.classes.push({ declared: newNameMap(), refs: [] });
}

// Declare `#name` in the innermost class body. False when it is already
// declared, unless one is a getter and the other a setter, both static
// or both not.
export function declarePrivate(ee, name, kind, isStatic) {
  const c = ee.classes[ee.classes.length - 1];
  const old = nameMapGet(c.declared, name);
  if (old === null) {
    nameMapSet(c.declared, name, { kind: kind, isStatic: isStatic });
    return true;
  }
  if (old.isStatic === isStatic && ((old.kind === PRIV_GET && kind === PRIV_SET) || (old.kind === PRIV_SET && kind === PRIV_GET))) {
    old.kind = PRIV_GETSET;
    return true;
  }
  return false;
}

// A use of a private name (the node holds the name and its place). False
// when no class body encloses it.
export function referencePrivate(ee, node) {
  if (ee.classes.length === 0) {
    return false;
  }
  ee.classes[ee.classes.length - 1].refs.push(node);
  return true;
}

// Close the innermost class body: its references to names it does not
// declare pass to the class around it. Returns the first reference no
// class declares, or null.
export function closeClass(ee) {
  const c = ee.classes.pop();
  let i = 0;
  while (i < c.refs.length) {
    const r = c.refs[i];
    if (nameMapGet(c.declared, r.name) === null) {
      if (ee.classes.length === 0) {
        return r;
      }
      ee.classes[ee.classes.length - 1].refs.push(r);
    }
    i = i + 1;
  }
  return null;
}

// ---- module exports ----

// An exported name (code units of the name or string). False when the
// module already exports it.
export function addExport(ee, name) {
  return nameSetAdd(ee.exported, name);
}

// `export { x }` without `from`: x must be declared in the module.
export function exportLocal(ee, ident) {
  ee.exportLocals.push(ident);
}

// At the end of a module: the first local export that names nothing
// declared at its top level, or null.
export function checkExportLocals(ee) {
  const s = ee.scopes[0];
  let i = 0;
  while (i < ee.exportLocals.length) {
    const id = ee.exportLocals[i];
    if (!has(s.lexical, id.name) && !has(s.vars, id.name)) {
      return id;
    }
    i = i + 1;
  }
  return null;
}
