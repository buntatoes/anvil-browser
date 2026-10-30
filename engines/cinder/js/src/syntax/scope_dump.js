// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// The canonical text form of a scope analysis (src/syntax/scope.js), as
// S-expressions: the functions, the scope tree with its bindings, and
// every reference in source order. Flags are printed only when set, in a
// fixed order. The format is described in docs/SCOPE.md.

import { quoteUnits } from "./token_dump.js";
import {
  RES_LOCAL,
  RES_CAPTURED,
  R_READ,
  R_WRITE,
  R_READ_WRITE,
  R_EXPORT,
} from "./scope.js";

const SCOPE_KINDS = [
  "global",
  "module",
  "function",
  "body",
  "function-name",
  "block",
  "catch",
  "class",
  "class-body",
  "for",
  "switch",
  "with",
];
const BINDING_KINDS = ["var", "let", "const", "class", "function", "param", "catch", "import", "arguments", "function-name", "class-name", "private"];
const FUNC_KINDS = ["script", "module", "function", "arrow", "method", "getter", "setter", "constructor", "field", "static-block"];
const PRIVATE_KINDS = ["field", "method", "getter", "setter", "accessor"];

export function scopeKindName(k) {
  return SCOPE_KINDS[k];
}

export function bindingKindName(k) {
  return BINDING_KINDS[k];
}

export function funcKindName(k) {
  return FUNC_KINDS[k];
}

function at(n) {
  return `@${n.line}:${n.col}`;
}

function flag(on, text) {
  if (on) {
    return " " + text;
  }
  return "";
}

function idList(prefix, ids) {
  if (ids.length === 0) {
    return "";
  }
  let s = ` ${prefix}=(`;
  let i = 0;
  while (i < ids.length) {
    if (i > 0) {
      s += " ";
    }
    s += `#${ids[i]}`;
    i += 1;
  }
  return s + ")";
}

function funcLine(a, f) {
  let s = `(Func #${f.id} ${FUNC_KINDS[f.kind]} ${at(f.node)}`;
  if (f.parent >= 0) {
    s += ` parent=#${f.parent}`;
  }
  s += ` scope=${f.scope}`;
  if (f.varScope !== f.scope) {
    s += ` var_scope=${f.varScope}`;
  }
  s += flag(f.strict, "strict");
  if (f.kind !== 0 && f.kind !== 1) {
    if (f.paramExpressions) {
      s += " params=expressions";
    } else if (!f.simpleParams) {
      s += " params=patterns";
    }
  }
  s += flag(f.usesThis, "this");
  s += flag(f.thisFromArrows, "this_from_arrows");
  s += flag(f.usesNewTarget, "new_target");
  s += flag(f.usesSuperProperty, "super_property");
  s += flag(f.usesSuperCall, "super_call");
  if (f.argumentsBinding >= 0) {
    s += ` arguments=#${f.argumentsBinding}`;
    s += flag(f.mapped, "mapped");
    s += flag(f.usesArguments, "used");
  }
  s += flag(f.directEval, "eval");
  s += flag(f.sloppyEval, "sloppy_eval");
  s += flag(f.evalInside && !f.directEval, "eval_inside");
  s += idList("captures", f.captures);
  return s + ")";
}

// `#` and the name, for private names.
export function privateUnits(name) {
  const u = [0x23];
  let i = 0;
  while (i < name.length) {
    u.push(name[i]);
    i += 1;
  }
  return u;
}

function bindingLine(a, b) {
  let name = b.name;
  if (b.privateKind >= 0) {
    name = privateUnits(name);
  }
  let s = `(Binding #${b.id} ${quoteUnits(name)} ${BINDING_KINDS[b.kind]} ${at(b.decl)}`;
  if (b.privateKind >= 0) {
    s += ` ${PRIVATE_KINDS[b.privateKind]}`;
    s += flag(b.isStatic, "static");
  }
  s += flag(b.tdz, "tdz");
  s += flag(b.captured, "captured");
  s += idList("by", b.capturedBy);
  s += flag(b.written, "written");
  s += flag(b.mutatedAfterCapture, "mutated_after_capture");
  s += flag(b.loopFresh, "loop_fresh");
  s += flag(b.exported, "exported");
  s += flag(b.argumentsObject, "arguments_object");
  if (b.annexVar >= 0) {
    s += ` annex=#${b.annexVar}`;
  }
  if (b.fromParam >= 0) {
    s += ` from_param=#${b.fromParam}`;
  }
  if (b.refCount === 0) {
    s += " unused";
  }
  return s + ")";
}

// The lines of scope id and its children; the closing parenthesis goes
// on the last line.
function scopeLines(a, id, indent, lines) {
  const sc = a.scopes[id];
  let s = `${indent}(Scope ${id} ${SCOPE_KINDS[sc.kind]} ${at(sc.node)} func=#${sc.func}`;
  s += flag(sc.strict, "strict");
  s += flag(sc.grows, "grows");
  s += flag(sc.evalVisible, "eval_visible");
  if (sc.funcInits.length > 0) {
    s += " inits=(";
    let i = 0;
    while (i < sc.funcInits.length) {
      if (i > 0) {
        s += " ";
      }
      s += at(sc.funcInits[i]);
      i += 1;
    }
    s += ")";
  }
  lines.push(s);
  const inner = indent + "  ";
  let i = 0;
  while (i < sc.bindings.length) {
    lines.push(inner + bindingLine(a, a.bindings[sc.bindings[i]]));
    i += 1;
  }
  i = 0;
  while (i < sc.children.length) {
    scopeLines(a, sc.children[i], inner, lines);
    i += 1;
  }
  lines[lines.length - 1] += ")";
}

function refKind(r) {
  if (r.kind === R_READ) {
    return "read";
  }
  if (r.kind === R_WRITE) {
    return "write";
  }
  if (r.kind === R_READ_WRITE) {
    return "read_write";
  }
  if (r.kind === R_EXPORT) {
    return "export";
  }
  return "private";
}

function refLine(r) {
  let name = r.name;
  if (r.kind !== R_READ && r.kind !== R_WRITE && r.kind !== R_READ_WRITE && r.kind !== R_EXPORT) {
    name = privateUnits(name);
  }
  let s = `(Ref ${quoteUnits(name)} ${at(r.node)} ${refKind(r)} scope=${r.scope}`;
  if (r.binding < 0) {
    s += " -> global";
  } else if (r.resolution === RES_LOCAL) {
    s += ` -> #${r.binding} local`;
  } else if (r.resolution === RES_CAPTURED) {
    s += ` -> #${r.binding} captured`;
  }
  s += flag(r.tdzCheck, "tdz_check");
  s += flag(r.dynamic, "dynamic");
  s += flag(r.typeofRef, "typeof");
  s += flag(r.deleteRef, "delete");
  s += flag(r.declInit, "init");
  return s + ")";
}

function smaller(x, y) {
  if (x < y) {
    return x;
  }
  return y;
}

// References in source order (by start offset, then as recorded).
function sortedRefs(refs) {
  let list = [];
  let i = 0;
  while (i < refs.length) {
    list.push(refs[i]);
    i += 1;
  }
  let width = 1;
  while (width < list.length) {
    const next = [];
    let lo = 0;
    while (lo < list.length) {
      const mid = smaller(lo + width, list.length);
      const hi = smaller(lo + 2 * width, list.length);
      let x = lo;
      let y = mid;
      while (x < mid || y < hi) {
        if (y >= hi || (x < mid && list[x].node.start <= list[y].node.start)) {
          next.push(list[x]);
          x += 1;
        } else {
          next.push(list[y]);
          y += 1;
        }
      }
      lo = hi;
    }
    list = next;
    width *= 2;
  }
  return list;
}

// Dump analysis a; calls out(line) for each line.
export function dumpScopes(a, out) {
  const lines = [];
  if (a.isModule) {
    lines.push("(Scopes module");
  } else {
    lines.push("(Scopes script");
  }
  lines.push("  (funcs");
  let i = 0;
  while (i < a.funcs.length) {
    lines.push("    " + funcLine(a, a.funcs[i]));
    i += 1;
  }
  lines[lines.length - 1] += ")";
  scopeLines(a, 0, "  ", lines);
  const refs = sortedRefs(a.refs);
  if (refs.length > 0) {
    lines.push("  (refs");
    i = 0;
    while (i < refs.length) {
      lines.push("    " + refLine(refs[i]));
      i += 1;
    }
    lines[lines.length - 1] += ")";
  }
  lines[lines.length - 1] += ")";
  i = 0;
  while (i < lines.length) {
    out(lines[i]);
    i += 1;
  }
}
