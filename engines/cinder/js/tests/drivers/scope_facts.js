// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Scope facts driver for `bundle scopediff`: the tool writes
// ./scope_input.js with scripts (no classes, no `with`); this prints
// `FILE k` and, per function, the facts `refdump --scopes-batch` prints
// for the reference's scope pass (format in tools/refdump/FORMAT.md),
// computed from src/syntax/scope.js and brought to the reference's
// model: one scope for parameters and body, a function expression's own
// name inside the function, parameters of a mapped `arguments` object
// counted as captured.
// needs: host

import { SOURCES, MODULE_GOAL } from "./scope_input.js";
import { createParser, parseProgram } from "../../src/syntax/parser.js";
import { parseError } from "../../src/syntax/parse_messages.js";
import { formatError } from "../../src/core/errors.js";
import {
  analyzeProgram,
  SC_BODY,
  SC_FUNC_NAME,
  B_VAR,
  B_LET,
  B_CONST,
  B_FUNCTION,
  B_PARAM,
  B_CATCH,
  B_FUNC_NAME,
} from "../../src/syntax/scope.js";
import { nameMapGet } from "../../src/syntax/names.js";
import { byteOffsets, quoteUnits, unitsOf } from "../../src/syntax/token_dump.js";

function kindText(k) {
  if (k === B_VAR) {
    return "var";
  }
  if (k === B_LET) {
    return "let";
  }
  if (k === B_CONST) {
    return "const";
  }
  if (k === B_FUNCTION) {
    return "function";
  }
  if (k === B_PARAM) {
    return "param";
  }
  if (k === B_CATCH) {
    return "catch";
  }
  if (k === B_FUNC_NAME) {
    return "function-name";
  }
  return "";
}

function sortStrings(list) {
  for (let i = 1; i < list.length; i++) {
    const x = list[i];
    let j = i - 1;
    while (j >= 0 && list[j] > x) {
      list[j + 1] = list[j];
      j -= 1;
    }
    list[j + 1] = x;
  }
}

function capturedOutside(nb, f) {
  for (const g of nb.capturedBy) {
    if (g !== f) {
      return true;
    }
  }
  return false;
}

function line(name, kind, captured) {
  let c = "-";
  if (captured) {
    c = "captured";
  }
  return `  ${quoteUnits(name)} ${kind} ${c}`;
}

function facts(a, bytes) {
  // Functions by start, then in walk order (insertion sort).
  const order = [];
  for (const f of a.funcs) {
    let j = order.length - 1;
    order.push(f);
    while (j >= 0 && bytes[order[j].node.start] > bytes[f.node.start]) {
      order[j + 1] = order[j];
      j -= 1;
    }
    order[j + 1] = f;
  }
  for (const f of order) {
    const usesArgs = f.argumentsBinding >= 0 && a.bindings[f.argumentsBinding].refCount > 0;
    const mapped = usesArgs && f.mapped;
    const text = [];
    // Parameter id -> captured, with a body copy's captures merged in.
    const params = new Map();
    const paramOrder = [];
    for (const b of a.bindings) {
      const sc = a.scopes[b.scope];
      const kind = kindText(b.kind);
      if (sc.func !== f.id || sc.kind === SC_FUNC_NAME || kind === "") {
        continue;
      }
      if (sc.kind === SC_BODY && b.fromParam >= 0 && a.bindings[b.fromParam].kind === B_PARAM) {
        if (!params.has(b.fromParam)) {
          paramOrder.push(b.fromParam);
          params.set(b.fromParam, false);
        }
        if (b.captured) {
          params.set(b.fromParam, true);
        }
        continue;
      }
      if (b.kind === B_PARAM) {
        if (!params.has(b.id)) {
          paramOrder.push(b.id);
          params.set(b.id, false);
        }
        if (b.captured) {
          params.set(b.id, true);
        }
        continue;
      }
      text.push(line(b.name, kind, b.captured));
    }
    for (const id of paramOrder) {
      text.push(line(a.bindings[id].name, "param", params.get(id) || mapped));
    }
    const outer = a.scopes[f.scope].parent;
    if (outer >= 0 && a.scopes[outer].kind === SC_FUNC_NAME && a.scopes[outer].node === f.node) {
      const nb = a.bindings[a.scopes[outer].bindings[0]];
      let shadowed = false;
      const own = [f.scope, f.varScope];
      for (const s of own) {
        const b = nameMapGet(a.scopes[s].names, nb.name);
        if (b !== null && !a.bindings[b].annexOnly) {
          shadowed = true;
        }
      }
      if (!shadowed) {
        text.push(line(nb.name, "function-name", capturedOutside(nb, f.id)));
      }
    }
    sortStrings(text);
    console.log(
      `func ${bytes[f.node.start]} arrow=${f.arrow} strict=${f.strict} simple_params=${f.simpleParams} arguments=${usesArgs} mapped=${mapped} this_from_arrows=${f.thisWordFromArrows}`,
    );
    for (const l of text) {
      console.log(l);
    }
  }
}

let k = 0;
while (k < SOURCES.length) {
  console.log(`FILE ${k}`);
  const units = unitsOf(SOURCES[k]);
  const p = createParser(units, MODULE_GOAL[k]);
  const prog = parseProgram(p);
  if (prog === null) {
    console.log(`SKIP ${formatError(parseError(p))}`);
  } else {
    const bytes = byteOffsets(units);
    facts(analyzeProgram(prog), bytes);
  }
  k += 1;
}
