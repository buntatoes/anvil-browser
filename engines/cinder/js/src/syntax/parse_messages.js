// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Text of the parser's errors, and the error object for a failed parse.
// The messages are the reference front end's, word for word, at the same
// places; lexer errors met while parsing keep the lexer's text.

import { makeError, SYNTAX_ERROR } from "../core/errors.js";
import { TK_IDENT, TK_PRIVATE, TK_PUNCT, TK_NUMBER, TK_BIGINT, TK_STRING, TK_TEMPLATE, TK_REGEX, PUNCT_TEXT, tokenName } from "./tokens.js";
import { WORD_UNITS } from "./words.js";
import { lexError } from "./lex_messages.js";
import { PE_NONE, PE_LEX, PARSE_ERROR_COUNT } from "./parser.js";

// By error code (see parser.js): text, the kind of a variable part, more
// text, the kind of a second variable part, and the end (kinds are empty
// where a message has fewer parts).
const MESSAGES = [
  ["", "", "", "", ""], // PE_NONE
  ["", "lex", "", "", ""], // PE_LEX
  ["expected `", "punct", "` but found ", "desc", ""], // PE_EXPECTED_PUNCT
  ["expected `", "word", "` but found ", "desc", ""], // PE_EXPECTED_WORD
  ["unexpected ", "desc", "", "", ""], // PE_UNEXPECTED
  ["nesting is too deep (limit ", "limit", ")", "", ""], // PE_DEPTH
  ["legacy octal literals and escapes are not allowed in strict mode", "", "", "", ""], // PE_LEGACY_OCTAL
  ["`", "name", "` is a reserved word", "", ""], // PE_RESERVED_WORD
  ["`", "name", "` is reserved in strict mode", "", ""], // PE_STRICT_RESERVED
  ["`yield` can't be a name inside a generator", "", "", "", ""], // PE_YIELD_IN_GENERATOR
  ["`await` can't be a name here", "", "", "", ""], // PE_AWAIT_NAME
  ["`", "name", "` can't be bound in strict mode", "", ""], // PE_CANT_BIND_STRICT
  ["duplicate parameter `", "name", "`", "", ""], // PE_DUPLICATE_PARAM
  ["`let` can't be a lexical binding name", "", "", "", ""], // PE_LET_LEXICAL
  ["`#constructor` is not allowed", "", "", "", ""], // PE_PRIVATE_CONSTRUCTOR
  ["legacy octal escapes are not allowed in strict mode", "", "", "", ""], // PE_OCTAL_DIRECTIVE
  ["`with` is not allowed in strict mode", "", "", "", ""], // PE_WITH_STRICT
  ["`return` is only allowed inside a function", "", "", "", ""], // PE_RETURN_OUTSIDE
  ["line break is not allowed after `throw`", "", "", "", ""], // PE_THROW_NEWLINE
  ["a function declaration is not allowed here", "", "", "", ""], // PE_FUNCTION_DECL_HERE
  ["a declaration is not allowed here", "", "", "", ""], // PE_DECL_HERE
  ["a statement can't start with `let [`", "", "", "", ""], // PE_LET_BRACKET
  ["a lexical declaration is not allowed here", "", "", "", ""], // PE_LEXICAL_HERE
  ["an async function declaration is not allowed here", "", "", "", ""], // PE_ASYNC_FUNCTION_HERE
  ["label `", "name", "` is already in use", "", ""], // PE_LABEL_IN_USE
  ["labeled function declarations are not allowed in strict mode", "", "", "", ""], // PE_LABELED_FUNCTION_STRICT
  ["only plain functions can be labeled", "", "", "", ""], // PE_ONLY_PLAIN_LABELED
  ["label `", "name", "` is not defined", "", ""], // PE_LABEL_NOT_DEFINED
  ["`continue` label `", "name", "` is not on a loop", "", ""], // PE_CONTINUE_NOT_LOOP
  ["`break` must be inside a loop or switch", "", "", "", ""], // PE_BREAK_OUTSIDE
  ["`continue` must be inside a loop", "", "", "", ""], // PE_CONTINUE_OUTSIDE
  ["a generator declaration is not allowed here", "", "", "", ""], // PE_GENERATOR_DECL_HERE
  ["a switch can only have one `default`", "", "", "", ""], // PE_SWITCH_DEFAULT
  ["duplicate catch parameter `", "name", "`", "", ""], // PE_DUPLICATE_CATCH_PARAM
  ["`try` needs a `catch` or `finally`", "", "", "", ""], // PE_TRY_NEEDS
  ["`const` declarations need an initializer", "", "", "", ""], // PE_CONST_INIT
  ["a destructuring declaration needs an initializer", "", "", "", ""], // PE_DESTRUCTURING_INIT
  ["`for await` is only allowed in async code", "", "", "", ""], // PE_FOR_AWAIT_ASYNC
  ["`for await` needs `of`", "", "", "", ""], // PE_FOR_AWAIT_OF
  ["only one binding is allowed in a for-in or for-of head", "", "", "", ""], // PE_FOR_ONE_BINDING
  ["a for-in or for-of binding can't have an initializer", "", "", "", ""], // PE_FOR_INIT
  ["a for-of target can't start with `let`", "", "", "", ""], // PE_FOR_OF_LET
  ["`for (async of` is not allowed", "", "", "", ""], // PE_FOR_ASYNC_OF
  ["a function declaration needs a name", "", "", "", ""], // PE_FUNCTION_NEEDS_NAME
  ["`yield` and `await` expressions are not allowed in parameters", "", "", "", ""], // PE_YIELD_AWAIT_PARAMS
  ["a getter takes no parameters", "", "", "", ""], // PE_GETTER_PARAMS
  ["a setter takes exactly one parameter", "", "", "", ""], // PE_SETTER_PARAMS
  ["\"use strict\" is not allowed in a function with non-simple parameters", "", "", "", ""], // PE_USE_STRICT_NON_SIMPLE
  ["a rest parameter can't have a default", "", "", "", ""], // PE_REST_PARAM_DEFAULT
  ["a rest parameter must be last", "", "", "", ""], // PE_REST_PARAM_LAST
  ["line break is not allowed before `=>`", "", "", "", ""], // PE_ARROW_NEWLINE
  ["`yield` and `await` expressions are not allowed in arrow parameters", "", "", "", ""], // PE_YIELD_AWAIT_ARROW_PARAMS
  ["`await` can't be used in async arrow parameters", "", "", "", ""], // PE_AWAIT_ASYNC_ARROW_PARAMS
  ["`await` can't be a parameter name here", "", "", "", ""], // PE_AWAIT_PARAM_NAME
  ["a class declaration needs a name", "", "", "", ""], // PE_CLASS_NEEDS_NAME
  ["unexpected arrow function", "", "", "", ""], // PE_UNEXPECTED_ARROW
  ["a class can only have one constructor", "", "", "", ""], // PE_ONE_CONSTRUCTOR
  ["the constructor can't be a getter, setter, async, or generator", "", "", "", ""], // PE_CONSTRUCTOR_SPECIAL
  ["a static method can't be named `prototype`", "", "", "", ""], // PE_STATIC_PROTOTYPE_METHOD
  ["a field can't be named `constructor`", "", "", "", ""], // PE_FIELD_CONSTRUCTOR
  ["a static field can't be named `prototype`", "", "", "", ""], // PE_STATIC_PROTOTYPE_FIELD
  ["a rest element must be last", "", "", "", ""], // PE_REST_ELEMENT_LAST
  ["an object rest element must be a plain name", "", "", "", ""], // PE_OBJECT_REST_PLAIN_NAME
  ["expected `:` after the property name", "", "", "", ""], // PE_EXPECTED_COLON
  ["put parentheses around a unary expression before `**`", "", "", "", ""], // PE_EXP_UNARY
  ["an arrow function can't be an operand here", "", "", "", ""], // PE_ARROW_OPERAND
  ["`??` can't be mixed with `&&` or `||` without parentheses", "", "", "", ""], // PE_NULLISH_MIX
  ["a private name here must be followed by `in`", "", "", "", ""], // PE_PRIVATE_IN
  ["deleting a plain name is not allowed in strict mode", "", "", "", ""], // PE_DELETE_NAME
  ["private fields can't be deleted", "", "", "", ""], // PE_DELETE_PRIVATE
  ["can't assign to `", "name", "` in strict mode", "", ""], // PE_ASSIGN_STRICT
  ["invalid assignment target", "", "", "", ""], // PE_INVALID_TARGET
  ["invalid destructuring target", "", "", "", ""], // PE_INVALID_DESTRUCTURING
  ["a rest element can't have a default", "", "", "", ""], // PE_REST_ELEMENT_DEFAULT
  ["an object rest element must be a name", "", "", "", ""], // PE_OBJECT_REST_NAME
  ["a method can't be a destructuring target", "", "", "", ""], // PE_METHOD_DESTRUCTURING
  ["invalid binding target", "", "", "", ""], // PE_INVALID_BINDING
  ["`super()` is only allowed in a derived class constructor", "", "", "", ""], // PE_SUPER_CALL
  ["`super` properties are only allowed in methods", "", "", "", ""], // PE_SUPER_PROP
  ["`super` must be followed by `(`, `.`, or `[`", "", "", "", ""], // PE_SUPER_FOLLOW
  ["expected `new.target`", "", "", "", ""], // PE_EXPECTED_NEW_TARGET
  ["`new.target` is only allowed in functions", "", "", "", ""], // PE_NEW_TARGET_OUTSIDE
  ["`new import(...)` is not allowed", "", "", "", ""], // PE_NEW_IMPORT
  ["an arrow function can't be used with `new`", "", "", "", ""], // PE_ARROW_NEW
  ["optional chaining is not allowed in a `new` callee", "", "", "", ""], // PE_OPTIONAL_IN_NEW
  ["`super?.` is not allowed", "", "", "", ""], // PE_SUPER_OPTIONAL
  ["a tagged template can't follow `?.`", "", "", "", ""], // PE_TAGGED_OPTIONAL
  ["expected `import.meta`", "", "", "", ""], // PE_EXPECTED_IMPORT_META
  ["`import.meta` is only allowed in modules", "", "", "", ""], // PE_IMPORT_META_MODULE
  ["expected `(` after `import`", "", "", "", ""], // PE_EXPECTED_IMPORT_PAREN
  ["`arguments` is not allowed in class field initializers or static blocks", "", "", "", ""], // PE_ARGUMENTS_CLASS_INIT
  ["expected `=>` after async arrow parameter", "", "", "", ""], // PE_EXPECTED_ARROW_ASYNC_PARAM
  ["expected `=>` after the parameter list", "", "", "", ""], // PE_EXPECTED_ARROW_PARAMS
  ["unexpected `...` in a parenthesized expression", "", "", "", ""], // PE_PAREN_SPREAD
  ["duplicate `__proto__` property", "", "", "", ""], // PE_DUPLICATE_PROTO
  ["`=` in an object literal is only allowed in destructuring", "", "", "", ""], // PE_COVER_INITIALIZER
  ["a keyword can't contain escapes", "", "", "", ""], // PE_KEYWORD_ESCAPES
  ["invalid escape in a template literal", "", "", "", ""], // PE_TEMPLATE_ESCAPE
  ["expected `}` to close the template substitution", "", "", "", ""], // PE_TEMPLATE_CLOSE
  ["`", "word", "` declarations are only allowed in modules", "", ""], // PE_MODULE_ONLY
  ["`", "word", "` declarations must be at the top level", "", ""], // PE_MODULE_TOP
  ["expected a string but found ", "desc", "", "", ""], // PE_EXPECTED_STRING
  ["a module export name must be well-formed Unicode", "", "", "", ""], // PE_EXPORT_NAME_UNICODE
  ["duplicate import attribute `", "name", "`", "", ""], // PE_DUPLICATE_ATTRIBUTE
  ["a string import name needs `as`", "", "", "", ""], // PE_STRING_IMPORT_AS
  ["a string can only be exported with `from`", "", "", "", ""], // PE_STRING_EXPORT_FROM
  ["expected a declaration after `export`", "", "", "", ""], // PE_EXPECTED_DECLARATION
  ["`", "name", "` has already been declared", "", ""], // PE_REDECLARED
  ["duplicate private name `#", "name", "`", "", ""], // PE_DUPLICATE_PRIVATE
  ["private name `#", "name", "` is not defined", "", ""], // PE_PRIVATE_NOT_DEFINED
  ["`", "name", "` is exported more than once", "", ""], // PE_EXPORTED_TWICE
  ["exported name `", "name", "` is not declared", "", ""], // PE_EXPORT_NOT_DECLARED
];

function text(units) {
  let s = "";
  let i = 0;
  while (i < units.length) {
    s += String.fromCharCode(units[i]);
    i += 1;
  }
  return s;
}

// How the reference names token i in a message.
export function describeToken(p, i) {
  const t = p.t;
  const k = t.kind[i];
  if (k === TK_IDENT) {
    return "`" + text(tokenName(t, p.src, i)) + "`";
  }
  if (k === TK_PRIVATE) {
    return "`#" + text(tokenName(t, p.src, i)) + "`";
  }
  if (k === TK_PUNCT) {
    return "`" + text(PUNCT_TEXT[t.aux[i]]) + "`";
  }
  if (k === TK_NUMBER || k === TK_BIGINT) {
    return "number";
  }
  if (k === TK_STRING) {
    return "string";
  }
  if (k === TK_TEMPLATE) {
    return "template";
  }
  if (k === TK_REGEX) {
    return "regular expression";
  }
  return "end of input";
}

// The message for the parser's error (empty when there is none).
export function parseErrorMessage(p) {
  const code = p.err;
  if (code === PE_NONE) {
    return "";
  }
  if (code === PE_LEX) {
    return lexError(p.lx).message;
  }
  const m = MESSAGES[code];
  return m[0] + part(p, m[1]) + m[2] + part(p, m[3]) + m[4];
}

function part(p, kind) {
  if (kind === "name") {
    return text(p.errName);
  }
  if (kind === "desc") {
    return describeToken(p, p.errTok);
  }
  if (kind === "punct") {
    return text(PUNCT_TEXT[p.errArg]);
  }
  if (kind === "word") {
    return text(WORD_UNITS[p.errArg]);
  }
  if (kind === "limit") {
    return `${p.errArg}`;
  }
  return "";
}

// The parser's error as an engine error object (see core/errors.js), or
// null when there is none.
export function parseError(p) {
  if (p.err === PE_NONE) {
    return null;
  }
  if (p.err === PE_LEX) {
    return lexError(p.lx);
  }
  return makeError(SYNTAX_ERROR, parseErrorMessage(p), p.errLine, p.errCol, p.errPos, p.errPos);
}

// Whether every error code has a message.
export const MESSAGES_COMPLETE = MESSAGES.length === PARSE_ERROR_COUNT;
