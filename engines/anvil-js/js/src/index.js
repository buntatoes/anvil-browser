// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Top-level entry point of the self-hosted Anvil JavaScript engine:
// parses ES2022+ scripts and modules, runs scope analysis, lowers all
// high-level constructs to Core JS, and emits executable Core JS source
// for AnvilVM.

import { formatError } from "./core/errors.js";
import { unitsOf } from "./syntax/token_dump.js";
import { createParser, parseProgram } from "./syntax/parser.js";
import { parseError } from "./syntax/parse_messages.js";
import { analyzeProgram } from "./syntax/scope.js";
import { emitProgram } from "./lower/emit.js";
import { lowerProgram } from "./lower/lower.js";
import { RUNTIME_PRELUDE_SOURCE } from "./runtime/builtins.js";
import {
  anvilJsonParse,
  anvilJsonStringify,
  anvilBtoa,
  anvilAtob,
  anvilEncodeURIComponent,
  anvilDecodeURIComponent,
} from "./runtime/json_codec.js";
import { createBrowserEnv } from "./runtime/web_apis.js";

function needsRuntimePrelude(code) {
  return (
    code.indexOf("__anvil_slice") >= 0 ||
    code.indexOf("__anvil_assign") >= 0 ||
    code.indexOf("__anvil_keys") >= 0
  );
}

export function compileSource(sourceText, isModule, forcePrelude) {
  const units = unitsOf(sourceText);
  const p = createParser(units, !!isModule);
  const prog = parseProgram(p);
  if (prog === null) {
    return {
      ok: false,
      code: "",
      error: formatError(parseError(p)),
    };
  }
  const lowered = lowerProgram(prog);
  let code = emitProgram(lowered);
  if (forcePrelude || needsRuntimePrelude(code)) {
    code = RUNTIME_PRELUDE_SOURCE + code;
  }
  return {
    ok: true,
    code: code,
    error: "",
  };
}

export function transpileToCoreJs(sourceText) {
  const res = compileSource(sourceText, false, false);
  if (!res.ok) {
    throw new Error(res.error);
  }
  return res.code;
}

export function parseAndAnalyze(sourceText, isModule) {
  const units = unitsOf(sourceText);
  const p = createParser(units, !!isModule);
  const prog = parseProgram(p);
  if (prog === null) {
    return null;
  }
  const scope = analyzeProgram(prog);
  return {
    program: prog,
    scope: scope,
  };
}

export function createRuntimeToolkit() {
  return {
    parseJson: anvilJsonParse,
    stringifyJson: anvilJsonStringify,
    btoa: anvilBtoa,
    atob: anvilAtob,
    encodeURIComponent: anvilEncodeURIComponent,
    decodeURIComponent: anvilDecodeURIComponent,
    createBrowserEnv: createBrowserEnv,
  };
}
