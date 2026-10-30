// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

// Self-hosted Core JS JSON parser, serializer, Base64 codec, and URI
// component encoder/decoder for the Anvil JavaScript engine.

function isWsCode(c) {
  return c === 32 || c === 9 || c === 10 || c === 13;
}

function isDigitCode(c) {
  return c >= 48 && c <= 57;
}

function hexVal(c) {
  if (c >= 48 && c <= 57) return c - 48;
  if (c >= 65 && c <= 70) return c - 55;
  if (c >= 97 && c <= 102) return c - 87;
  return -1;
}

function hexChar(n) {
  const digits = "0123456789ABCDEF";
  return digits[n & 15];
}

function hexCharLower(n) {
  const digits = "0123456789abcdef";
  return digits[n & 15];
}

function toStr(x) {
  return "" + x;
}

function isArrayVal(val) {
  return (
    val !== null &&
    typeof val === "object" &&
    typeof val.length === "number" &&
    val._keys === undefined &&
    val.nodeType === undefined
  );
}

export function anvilJsonStringify(val) {
  if (val === null || val === undefined) {
    return "null";
  }
  const t = typeof val;
  if (t === "boolean") {
    return val ? "true" : "false";
  }
  if (t === "number") {
    if (val !== val || val === Infinity || val === -Infinity) {
      return "null";
    }
    return toStr(val);
  }
  if (t === "string") {
    return quoteJsonString(val);
  }
  if (isArrayVal(val)) {
    let out = "[";
    for (let i = 0; i < val.length; i++) {
      if (i > 0) out = out + ",";
      out = out + anvilJsonStringify(val[i]);
    }
    return out + "]";
  }
  if (t === "object") {
    if (typeof val.toJSON === "function") {
      return anvilJsonStringify(val.toJSON());
    }
    const keys = isArrayVal(val._keys) ? val._keys : [];
    let out = "{";
    let first = true;
    for (let i = 0; i < keys.length; i++) {
      const k = toStr(keys[i]);
      const v = val[k];
      if (v === undefined || typeof v === "function") {
        continue;
      }
      if (!first) out = out + ",";
      first = false;
      out = out + quoteJsonString(k) + ":" + anvilJsonStringify(v);
    }
    return out + "}";
  }
  return "null";
}

function quoteJsonString(s) {
  const str = toStr(s);
  let out = '"';
  for (let i = 0; i < str.length; i++) {
    const c = str.charCodeAt(i);
    if (c === 34) {
      out = out + '\\"';
    } else if (c === 92) {
      out = out + "\\\\";
    } else if (c === 8) {
      out = out + "\\b";
    } else if (c === 12) {
      out = out + "\\f";
    } else if (c === 10) {
      out = out + "\\n";
    } else if (c === 13) {
      out = out + "\\r";
    } else if (c === 9) {
      out = out + "\\t";
    } else if (c < 32) {
      out =
        out +
        "\\u00" +
        hexCharLower((c >> 4) & 15) +
        hexCharLower(c & 15);
    } else {
      out = out + String.fromCharCode(c);
    }
  }
  return out + '"';
}

export function anvilJsonParse(text) {
  const s = toStr(text);
  const st = { s: s, pos: 0, len: s.length };
  skipJsonWs(st);
  const v = parseJsonValue(st);
  skipJsonWs(st);
  if (st.pos < st.len) {
    throw new Error("SyntaxError: Unexpected trailing token in JSON");
  }
  return v;
}

function skipJsonWs(st) {
  while (st.pos < st.len && isWsCode(st.s.charCodeAt(st.pos))) {
    st.pos = st.pos + 1;
  }
}

function parseJsonValue(st) {
  skipJsonWs(st);
  if (st.pos >= st.len) {
    throw new Error("SyntaxError: Unexpected end of JSON input");
  }
  const c = st.s.charCodeAt(st.pos);
  if (c === 34) {
    return parseJsonString(st);
  }
  if (c === 123) {
    return parseJsonObject(st);
  }
  if (c === 91) {
    return parseJsonArray(st);
  }
  if (c === 116) {
    if (st.s.slice(st.pos, st.pos + 4) === "true") {
      st.pos = st.pos + 4;
      return true;
    }
    throw new Error("SyntaxError: Unexpected token in JSON");
  }
  if (c === 102) {
    if (st.s.slice(st.pos, st.pos + 5) === "false") {
      st.pos = st.pos + 5;
      return false;
    }
    throw new Error("SyntaxError: Unexpected token in JSON");
  }
  if (c === 110) {
    if (st.s.slice(st.pos, st.pos + 4) === "null") {
      st.pos = st.pos + 4;
      return null;
    }
    throw new Error("SyntaxError: Unexpected token in JSON");
  }
  if (c === 45 || isDigitCode(c)) {
    return parseJsonNumber(st);
  }
  throw new Error("SyntaxError: Unexpected token in JSON");
}

function parseJsonString(st) {
  st.pos = st.pos + 1;
  let out = "";
  while (st.pos < st.len) {
    const c = st.s.charCodeAt(st.pos);
    st.pos = st.pos + 1;
    if (c === 34) {
      return out;
    }
    if (c === 92) {
      if (st.pos >= st.len) {
        throw new Error("SyntaxError: Unterminated escape in JSON string");
      }
      const esc = st.s.charCodeAt(st.pos);
      st.pos = st.pos + 1;
      if (esc === 34) out = out + '"';
      else if (esc === 92) out = out + "\\";
      else if (esc === 47) out = out + "/";
      else if (esc === 98) out = out + "\b";
      else if (esc === 102) out = out + "\f";
      else if (esc === 110) out = out + "\n";
      else if (esc === 114) out = out + "\r";
      else if (esc === 116) out = out + "\t";
      else if (esc === 117) {
        if (st.pos + 4 > st.len) {
          throw new Error("SyntaxError: Invalid unicode escape in JSON");
        }
        const h0 = hexVal(st.s.charCodeAt(st.pos));
        const h1 = hexVal(st.s.charCodeAt(st.pos + 1));
        const h2 = hexVal(st.s.charCodeAt(st.pos + 2));
        const h3 = hexVal(st.s.charCodeAt(st.pos + 3));
        if (h0 < 0 || h1 < 0 || h2 < 0 || h3 < 0) {
          throw new Error("SyntaxError: Invalid unicode escape in JSON");
        }
        st.pos = st.pos + 4;
        const code = (h0 << 12) | (h1 << 8) | (h2 << 4) | h3;
        out = out + String.fromCharCode(code);
      } else {
        out = out + String.fromCharCode(esc);
      }
    } else {
      out = out + String.fromCharCode(c);
    }
  }
  throw new Error("SyntaxError: Unterminated string in JSON");
}

function parseJsonNumber(st) {
  let neg = false;
  if (st.s.charCodeAt(st.pos) === 45) {
    neg = true;
    st.pos = st.pos + 1;
  }
  let intVal = 0;
  let digits = 0;
  while (st.pos < st.len && isDigitCode(st.s.charCodeAt(st.pos))) {
    intVal = intVal * 10 + (st.s.charCodeAt(st.pos) - 48);
    st.pos = st.pos + 1;
    digits = digits + 1;
  }
  if (digits === 0) {
    throw new Error("SyntaxError: Invalid number in JSON");
  }
  let fracVal = 0;
  let fracScale = 1;
  if (st.pos < st.len && st.s.charCodeAt(st.pos) === 46) {
    st.pos = st.pos + 1;
    while (st.pos < st.len && isDigitCode(st.s.charCodeAt(st.pos))) {
      fracVal = fracVal * 10 + (st.s.charCodeAt(st.pos) - 48);
      fracScale = fracScale * 10;
      st.pos = st.pos + 1;
    }
  }
  let n = intVal + fracVal / fracScale;
  if (st.pos < st.len) {
    const e = st.s.charCodeAt(st.pos);
    if (e === 101 || e === 69) {
      st.pos = st.pos + 1;
      let expNeg = false;
      if (st.pos < st.len) {
        const sign = st.s.charCodeAt(st.pos);
        if (sign === 43 || sign === 45) {
          expNeg = sign === 45;
          st.pos = st.pos + 1;
        }
      }
      let expVal = 0;
      while (st.pos < st.len && isDigitCode(st.s.charCodeAt(st.pos))) {
        expVal = expVal * 10 + (st.s.charCodeAt(st.pos) - 48);
        st.pos = st.pos + 1;
      }
      const factor = 10 ** expVal;
      n = expNeg ? n / factor : n * factor;
    }
  }
  return neg ? -n : n;
}

function parseJsonArray(st) {
  st.pos = st.pos + 1;
  const out = [];
  skipJsonWs(st);
  if (st.pos < st.len && st.s.charCodeAt(st.pos) === 93) {
    st.pos = st.pos + 1;
    return out;
  }
  while (st.pos < st.len) {
    out.push(parseJsonValue(st));
    skipJsonWs(st);
    if (st.pos < st.len && st.s.charCodeAt(st.pos) === 44) {
      st.pos = st.pos + 1;
      continue;
    }
    if (st.pos < st.len && st.s.charCodeAt(st.pos) === 93) {
      st.pos = st.pos + 1;
      return out;
    }
    throw new Error("SyntaxError: Expected ',' or ']' in JSON array");
  }
  throw new Error("SyntaxError: Unterminated JSON array");
}

function parseJsonObject(st) {
  st.pos = st.pos + 1;
  const keys = [];
  const out = { _keys: keys };
  skipJsonWs(st);
  if (st.pos < st.len && st.s.charCodeAt(st.pos) === 125) {
    st.pos = st.pos + 1;
    return out;
  }
  while (st.pos < st.len) {
    skipJsonWs(st);
    if (st.pos >= st.len || st.s.charCodeAt(st.pos) !== 34) {
      throw new Error("SyntaxError: Expected string key in JSON object");
    }
    const key = parseJsonString(st);
    skipJsonWs(st);
    if (st.pos >= st.len || st.s.charCodeAt(st.pos) !== 58) {
      throw new Error("SyntaxError: Expected ':' after key in JSON object");
    }
    st.pos = st.pos + 1;
    keys.push(key);
    out[key] = parseJsonValue(st);
    skipJsonWs(st);
    if (st.pos < st.len && st.s.charCodeAt(st.pos) === 44) {
      st.pos = st.pos + 1;
      continue;
    }
    if (st.pos < st.len && st.s.charCodeAt(st.pos) === 125) {
      st.pos = st.pos + 1;
      return out;
    }
    throw new Error("SyntaxError: Expected ',' or '}' in JSON object");
  }
  throw new Error("SyntaxError: Unterminated JSON object");
}

const B64_TABLE =
  "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

export function anvilBtoa(input) {
  const s = toStr(input);
  let out = "";
  let i = 0;
  while (i < s.length) {
    const b0 = s.charCodeAt(i++) & 255;
    const has1 = i < s.length;
    const b1 = has1 ? s.charCodeAt(i++) & 255 : 0;
    const has2 = i < s.length;
    const b2 = has2 ? s.charCodeAt(i++) & 255 : 0;
    const triplet = (b0 << 16) | (b1 << 8) | b2;
    out = out + B64_TABLE[(triplet >> 18) & 63];
    out = out + B64_TABLE[(triplet >> 12) & 63];
    out = out + (has1 ? B64_TABLE[(triplet >> 6) & 63] : "=");
    out = out + (has2 ? B64_TABLE[triplet & 63] : "=");
  }
  return out;
}

export function anvilAtob(input) {
  const s = toStr(input);
  let clean = "";
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    if (!isWsCode(c) && c !== 61) {
      clean = clean + String.fromCharCode(c);
    }
  }
  let out = "";
  let i = 0;
  while (i < clean.length) {
    const v0 = B64_TABLE.indexOf(clean[i++]);
    const v1 = i < clean.length ? B64_TABLE.indexOf(clean[i++]) : 0;
    const has2 = i < clean.length;
    const v2 = has2 ? B64_TABLE.indexOf(clean[i++]) : 0;
    const has3 = i < clean.length;
    const v3 = has3 ? B64_TABLE.indexOf(clean[i++]) : 0;
    const triplet = (v0 << 18) | (v1 << 12) | (v2 << 6) | v3;
    out = out + String.fromCharCode((triplet >> 16) & 255);
    if (has2) out = out + String.fromCharCode((triplet >> 8) & 255);
    if (has3) out = out + String.fromCharCode(triplet & 255);
  }
  return out;
}

function isUnescapedUriChar(c) {
  if (c >= 65 && c <= 90) return true;
  if (c >= 97 && c <= 122) return true;
  if (c >= 48 && c <= 57) return true;
  return (
    c === 45 ||
    c === 95 ||
    c === 46 ||
    c === 33 ||
    c === 126 ||
    c === 42 ||
    c === 39 ||
    c === 40 ||
    c === 41
  );
}

export function anvilEncodeURIComponent(str) {
  const s = toStr(str);
  let out = "";
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    if (isUnescapedUriChar(c)) {
      out = out + String.fromCharCode(c);
    } else if (c < 128) {
      out = out + "%" + hexChar((c >> 4) & 15) + hexChar(c & 15);
    } else if (c < 2048) {
      const b0 = 192 | (c >> 6);
      const b1 = 128 | (c & 63);
      out =
        out +
        "%" +
        hexChar((b0 >> 4) & 15) +
        hexChar(b0 & 15) +
        "%" +
        hexChar((b1 >> 4) & 15) +
        hexChar(b1 & 15);
    } else {
      const b0 = 224 | (c >> 12);
      const b1 = 128 | ((c >> 6) & 63);
      const b2 = 128 | (c & 63);
      out =
        out +
        "%" +
        hexChar((b0 >> 4) & 15) +
        hexChar(b0 & 15) +
        "%" +
        hexChar((b1 >> 4) & 15) +
        hexChar(b1 & 15) +
        "%" +
        hexChar((b2 >> 4) & 15) +
        hexChar(b2 & 15);
    }
  }
  return out;
}

export function anvilDecodeURIComponent(str) {
  const s = toStr(str);
  let out = "";
  let i = 0;
  while (i < s.length) {
    const c = s.charCodeAt(i);
    if (c === 37 && i + 2 < s.length) {
      const h0 = hexVal(s.charCodeAt(i + 1));
      const h1 = hexVal(s.charCodeAt(i + 2));
      if (h0 >= 0 && h1 >= 0) {
        const b0 = (h0 << 4) | h1;
        i = i + 3;
        if (b0 < 128) {
          out = out + String.fromCharCode(b0);
        } else if ((b0 & 224) === 192 && i + 2 < s.length && s.charCodeAt(i) === 37) {
          const b1 = (hexVal(s.charCodeAt(i + 1)) << 4) | hexVal(s.charCodeAt(i + 2));
          i = i + 3;
          out = out + String.fromCharCode(((b0 & 31) << 6) | (b1 & 63));
        } else if ((b0 & 240) === 224 && i + 5 < s.length) {
          const b1 = (hexVal(s.charCodeAt(i + 1)) << 4) | hexVal(s.charCodeAt(i + 2));
          const b2 = (hexVal(s.charCodeAt(i + 4)) << 4) | hexVal(s.charCodeAt(i + 5));
          i = i + 6;
          out =
            out +
            String.fromCharCode(((b0 & 15) << 12) | ((b1 & 63) << 6) | (b2 & 63));
        } else {
          out = out + String.fromCharCode(b0);
        }
        continue;
      }
    }
    if (c === 43) {
      out = out + " ";
    } else {
      out = out + String.fromCharCode(c);
    }
    i = i + 1;
  }
  return out;
}
