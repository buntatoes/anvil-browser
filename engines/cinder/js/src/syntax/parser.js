// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// The parser: tokens in, a syntax tree (ast.js) out, for scripts and
// modules (spec clauses 13 to 16, with annex B where the reference
// follows it). Recursive descent for statements, precedence climbing for
// binary operators, and the cover grammar for arrow parameters and
// destructuring assignment: such code is read as an expression first and
// turned into a pattern when `=>` or `=` shows what it was.
//
// The parser drives the two lexer choices that depend on the grammar: a
// `/` or `/=` where an expression starts is read again as a regular
// expression, and the `}` that ends a template substitution is read
// again as the next template piece.
//
// Errors: every function returns its node, or null once an error has been
// recorded in the state (p.err and the fields next to it); the first
// error stops the parse. parse_messages.js turns the record into the
// reference's message. Functions that may return null on success check
// p.err instead.
//
// Nesting: every level of recursion, and every link of a long chain such
// as `a.b.c` or `1 + 2 + 3`, counts one level against MAX_PARSE_DEPTH,
// at the same places as in the reference, so both reject the same inputs
// with "nesting is too deep". Long chains and runs of prefix operators
// are loops, so the parser's own call depth stays a small multiple of
// the nesting depth.
//
// Early errors that need the declarations of a scope, a class body or a
// module (redeclarations, private names, export lists) are checked as the
// parser reads, through early_errors.js.

import { sameUnits, newNameSet, nameSetAdd } from "./names.js";
import {
  newEarlyErrors,
  enterScope,
  exitScope,
  setCatchParams,
  setParams,
  declareLexical,
  declareVar,
  declareFunction,
  openClass,
  declarePrivate,
  referencePrivate,
  closeClass,
  addExport,
  exportLocal,
  checkExportLocals,
  PRIV_FIELD,
  PRIV_METHOD,
  PRIV_GET,
  PRIV_SET,
} from "./early_errors.js";
import {
  MAX_SOURCE_BYTES,
  MAX_PARSE_DEPTH,
} from "../core/limits.js";
import {
  lineIndex,
} from "../core/positions.js";
import {
  TK_EOF,
  TK_IDENT,
  TK_PRIVATE,
  TK_PUNCT,
  TK_NUMBER,
  TK_BIGINT,
  TK_STRING,
  TK_TEMPLATE,
  TK_REGEX,
  F_NL,
  F_ESCAPED,
  F_LEGACY_OCTAL,
  F_TAIL,
  P_LBRACE,
  P_RBRACE,
  P_LPAREN,
  P_RPAREN,
  P_LBRACKET,
  P_RBRACKET,
  P_DOT,
  P_ELLIPSIS,
  P_SEMICOLON,
  P_COMMA,
  P_LT,
  P_GT,
  P_LT_EQ,
  P_GT_EQ,
  P_EQ_EQ,
  P_NOT_EQ,
  P_EQ_EQ_EQ,
  P_NOT_EQ_EQ,
  P_PLUS,
  P_MINUS,
  P_STAR,
  P_PERCENT,
  P_STAR_STAR,
  P_PLUS_PLUS,
  P_MINUS_MINUS,
  P_SHL,
  P_SHR,
  P_USHR,
  P_AMP,
  P_PIPE,
  P_CARET,
  P_BANG,
  P_TILDE,
  P_AMP_AMP,
  P_PIPE_PIPE,
  P_QUESTION_QUESTION,
  P_QUESTION,
  P_QUESTION_DOT,
  P_COLON,
  P_EQ,
  P_PLUS_EQ,
  P_MINUS_EQ,
  P_STAR_EQ,
  P_PERCENT_EQ,
  P_STAR_STAR_EQ,
  P_SHL_EQ,
  P_SHR_EQ,
  P_USHR_EQ,
  P_AMP_EQ,
  P_PIPE_EQ,
  P_CARET_EQ,
  P_AMP_AMP_EQ,
  P_PIPE_PIPE_EQ,
  P_QUESTION_QUESTION_EQ,
  P_ARROW,
  P_SLASH,
  P_SLASH_EQ,
  sliceUnits,
  rangeIs,
  tokenName,
  stringValue,
  templateCooked,
  templateRaw,
  templateBadEscape,
  regexPattern,
  regexFlags,
  numberValue,
  bigintRadix,
  bigintDigits,
} from "./tokens.js";
import {
  E_NONE,
  createLexerWithLimit,
  nextToken,
  rescanRegex,
  rescanTemplate,
  rewind,
} from "./lexer.js";
import {
  W_BREAK,
  W_CASE,
  W_CATCH,
  W_CLASS,
  W_CONST,
  W_CONTINUE,
  W_DEBUGGER,
  W_DEFAULT,
  W_DELETE,
  W_DO,
  W_ELSE,
  W_EXPORT,
  W_EXTENDS,
  W_FALSE,
  W_FINALLY,
  W_FOR,
  W_FUNCTION,
  W_IF,
  W_IMPORT,
  W_IN,
  W_INSTANCEOF,
  W_NEW,
  W_NULL,
  W_RETURN,
  W_SUPER,
  W_SWITCH,
  W_THIS,
  W_THROW,
  W_TRUE,
  W_TRY,
  W_TYPEOF,
  W_VAR,
  W_VOID,
  W_WHILE,
  W_WITH,
  W_AWAIT,
  W_YIELD,
  W_LET,
  W_STATIC,
  W_AS,
  W_ASYNC,
  W_FROM,
  W_GET,
  W_META,
  W_OF,
  W_SET,
  W_TARGET,
  W_EVAL,
  W_ARGUMENTS,
  W_CONSTRUCTOR,
  W_PROTOTYPE,
  W_PROTO,
  wordId,
  isReservedWord,
  isStrictReservedWord,
} from "./words.js";
import {
  S_EXPR,
  S_VAR,
  S_FUNCTION,
  S_CLASS,
  CM_METHOD,
  PK_IDENT,
  PK_STR,
  PK_COMPUTED,
  PK_PRIVATE,
  E_IDENT,
  E_SUPER,
  E_STR,
  E_ARRAY,
  E_OBJECT,
  E_ARROW,
  E_UNARY,
  E_LOGICAL,
  E_ASSIGN,
  E_MEMBER,
  E_OPT_CHAIN,
  E_SPREAD,
  E_AWAIT,
  E_PAREN,
  MP_PRIVATE,
  PR_INIT,
  PR_SHORTHAND,
  PR_SPREAD,
  PT_IDENT,
  PT_ARRAY,
  PT_OBJECT,
  PT_ASSIGN,
  PT_REST,
  PT_EXPR,
  PP_PROP,
  MN_IDENT,
  MN_STR,
  U_MINUS,
  U_PLUS,
  U_NOT,
  U_BIT_NOT,
  U_TYPEOF,
  U_VOID,
  U_DELETE,
  UP_INC,
  UP_DEC,
  B_EQ,
  B_NOT_EQ,
  B_STRICT_EQ,
  B_STRICT_NOT_EQ,
  B_LT,
  B_LT_EQ,
  B_GT,
  B_GT_EQ,
  B_SHL,
  B_SHR,
  B_USHR,
  B_ADD,
  B_SUB,
  B_MUL,
  B_DIV,
  B_MOD,
  B_EXP,
  B_BIT_OR,
  B_BIT_XOR,
  B_BIT_AND,
  B_IN,
  B_INSTANCEOF,
  L_AND,
  L_OR,
  L_NULLISH,
  A_ASSIGN,
  A_ADD,
  A_SUB,
  A_MUL,
  A_DIV,
  A_MOD,
  A_EXP,
  A_SHL,
  A_SHR,
  A_USHR,
  A_BIT_AND,
  A_BIT_OR,
  A_BIT_XOR,
  A_AND,
  A_OR,
  A_NULLISH,
  MK_METHOD,
  MK_GET,
  MK_SET,
  MK_CONSTRUCTOR,
  VK_VAR,
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
  makeStmtForIn,
  makeStmtForOf,
  makeStmtWhile,
  makeStmtDoWhile,
  makeStmtContinue,
  makeStmtBreak,
  makeStmtReturn,
  makeStmtWith,
  makeStmtSwitch,
  makeStmtLabeled,
  makeStmtThrow,
  makeStmtTry,
  makeStmtDebugger,
  makeStmtImport,
  makeStmtExport,
  makeVarDecl,
  makeVarDeclarator,
  makeForInitVar,
  makeForInitExpr,
  makeForHeadVar,
  makeForHeadPat,
  makeSwitchCase,
  makeCatch,
  makeFunction,
  makeArrow,
  makeArrowBodyExpr,
  makeArrowBodyBlock,
  makeClass,
  makeMemberMethod,
  makeMemberField,
  makeMemberStaticBlock,
  makeKeyIdent,
  makeKeyStr,
  makeKeyNum,
  makeKeyBigint,
  makeKeyComputed,
  makeKeyPrivate,
  makeExprIdent,
  makeExprThis,
  makeExprSuper,
  makeExprNull,
  makeExprBool,
  makeExprNum,
  makeExprBigint,
  makeExprStr,
  makeExprRegex,
  makeExprTemplate,
  makeExprTagged,
  makeExprArray,
  makeExprObject,
  makeExprFunction,
  makeExprArrow,
  makeExprClass,
  makeExprUnary,
  makeExprUpdate,
  makeExprBinary,
  makeExprLogical,
  makeExprAssign,
  makeExprCond,
  makeExprCall,
  makeExprNew,
  makeExprMember,
  makeExprOptChain,
  makeExprSeq,
  makeExprSpread,
  makeExprYield,
  makeExprAwait,
  makeExprNewTarget,
  makeExprImportMeta,
  makeExprImport,
  makeExprParen,
  makeExprPrivateIn,
  makePropName,
  makePropComputed,
  makePropPrivate,
  makeTemplate,
  makeTemplateElement,
  makeObjPropInit,
  makeObjPropShorthand,
  makeObjPropMethod,
  makeObjPropSpread,
  makePatIdent,
  makePatArray,
  makePatObject,
  makePatAssign,
  makePatRest,
  makePatExpr,
  makePatPropProp,
  makePatPropRest,
  makeImportDecl,
  makeStrLit,
  makeImportAttribute,
  makeModuleNameIdent,
  makeModuleNameStr,
  makeImportSpecDefault,
  makeImportSpecNamespace,
  makeImportSpecNamed,
  makeExportNamed,
  makeExportAll,
  makeExportDecl,
  makeExportDefault,
  makeExportSpec,
  makeExportDefaultFunction,
  makeExportDefaultClass,
  makeExportDefaultExpr,
} from "./ast.js";

// Error codes; parse_messages.js has the text of each.
export const PE_NONE = 0;
export const PE_LEX = 1;
export const PE_EXPECTED_PUNCT = 2;
export const PE_EXPECTED_WORD = 3;
export const PE_UNEXPECTED = 4;
export const PE_DEPTH = 5;
export const PE_LEGACY_OCTAL = 6;
export const PE_RESERVED_WORD = 7;
export const PE_STRICT_RESERVED = 8;
export const PE_YIELD_IN_GENERATOR = 9;
export const PE_AWAIT_NAME = 10;
export const PE_CANT_BIND_STRICT = 11;
export const PE_DUPLICATE_PARAM = 12;
export const PE_LET_LEXICAL = 13;
export const PE_PRIVATE_CONSTRUCTOR = 14;
export const PE_OCTAL_DIRECTIVE = 15;
export const PE_WITH_STRICT = 16;
export const PE_RETURN_OUTSIDE = 17;
export const PE_THROW_NEWLINE = 18;
export const PE_FUNCTION_DECL_HERE = 19;
export const PE_DECL_HERE = 20;
export const PE_LET_BRACKET = 21;
export const PE_LEXICAL_HERE = 22;
export const PE_ASYNC_FUNCTION_HERE = 23;
export const PE_LABEL_IN_USE = 24;
export const PE_LABELED_FUNCTION_STRICT = 25;
export const PE_ONLY_PLAIN_LABELED = 26;
export const PE_LABEL_NOT_DEFINED = 27;
export const PE_CONTINUE_NOT_LOOP = 28;
export const PE_BREAK_OUTSIDE = 29;
export const PE_CONTINUE_OUTSIDE = 30;
export const PE_GENERATOR_DECL_HERE = 31;
export const PE_SWITCH_DEFAULT = 32;
export const PE_DUPLICATE_CATCH_PARAM = 33;
export const PE_TRY_NEEDS = 34;
export const PE_CONST_INIT = 35;
export const PE_DESTRUCTURING_INIT = 36;
export const PE_FOR_AWAIT_ASYNC = 37;
export const PE_FOR_AWAIT_OF = 38;
export const PE_FOR_ONE_BINDING = 39;
export const PE_FOR_INIT = 40;
export const PE_FOR_OF_LET = 41;
export const PE_FOR_ASYNC_OF = 42;
export const PE_FUNCTION_NEEDS_NAME = 43;
export const PE_YIELD_AWAIT_PARAMS = 44;
export const PE_GETTER_PARAMS = 45;
export const PE_SETTER_PARAMS = 46;
export const PE_USE_STRICT_NON_SIMPLE = 47;
export const PE_REST_PARAM_DEFAULT = 48;
export const PE_REST_PARAM_LAST = 49;
export const PE_ARROW_NEWLINE = 50;
export const PE_YIELD_AWAIT_ARROW_PARAMS = 51;
export const PE_AWAIT_ASYNC_ARROW_PARAMS = 52;
export const PE_AWAIT_PARAM_NAME = 53;
export const PE_CLASS_NEEDS_NAME = 54;
export const PE_UNEXPECTED_ARROW = 55;
export const PE_ONE_CONSTRUCTOR = 56;
export const PE_CONSTRUCTOR_SPECIAL = 57;
export const PE_STATIC_PROTOTYPE_METHOD = 58;
export const PE_FIELD_CONSTRUCTOR = 59;
export const PE_STATIC_PROTOTYPE_FIELD = 60;
export const PE_REST_ELEMENT_LAST = 61;
export const PE_OBJECT_REST_PLAIN_NAME = 62;
export const PE_EXPECTED_COLON = 63;
export const PE_EXP_UNARY = 64;
export const PE_ARROW_OPERAND = 65;
export const PE_NULLISH_MIX = 66;
export const PE_PRIVATE_IN = 67;
export const PE_DELETE_NAME = 68;
export const PE_DELETE_PRIVATE = 69;
export const PE_ASSIGN_STRICT = 70;
export const PE_INVALID_TARGET = 71;
export const PE_INVALID_DESTRUCTURING = 72;
export const PE_REST_ELEMENT_DEFAULT = 73;
export const PE_OBJECT_REST_NAME = 74;
export const PE_METHOD_DESTRUCTURING = 75;
export const PE_INVALID_BINDING = 76;
export const PE_SUPER_CALL = 77;
export const PE_SUPER_PROP = 78;
export const PE_SUPER_FOLLOW = 79;
export const PE_EXPECTED_NEW_TARGET = 80;
export const PE_NEW_TARGET_OUTSIDE = 81;
export const PE_NEW_IMPORT = 82;
export const PE_ARROW_NEW = 83;
export const PE_OPTIONAL_IN_NEW = 84;
export const PE_SUPER_OPTIONAL = 85;
export const PE_TAGGED_OPTIONAL = 86;
export const PE_EXPECTED_IMPORT_META = 87;
export const PE_IMPORT_META_MODULE = 88;
export const PE_EXPECTED_IMPORT_PAREN = 89;
export const PE_ARGUMENTS_CLASS_INIT = 90;
export const PE_EXPECTED_ARROW_ASYNC_PARAM = 91;
export const PE_EXPECTED_ARROW_PARAMS = 92;
export const PE_PAREN_SPREAD = 93;
export const PE_DUPLICATE_PROTO = 94;
export const PE_COVER_INITIALIZER = 95;
export const PE_KEYWORD_ESCAPES = 96;
export const PE_TEMPLATE_ESCAPE = 97;
export const PE_TEMPLATE_CLOSE = 98;
export const PE_MODULE_ONLY = 99;
export const PE_MODULE_TOP = 100;
export const PE_EXPECTED_STRING = 101;
export const PE_EXPORT_NAME_UNICODE = 102;
export const PE_DUPLICATE_ATTRIBUTE = 103;
export const PE_STRING_IMPORT_AS = 104;
export const PE_STRING_EXPORT_FROM = 105;
export const PE_EXPECTED_DECLARATION = 106;
export const PE_REDECLARED = 107;
export const PE_DUPLICATE_PRIVATE = 108;
export const PE_PRIVATE_NOT_DEFINED = 109;
export const PE_EXPORTED_TWICE = 110;
export const PE_EXPORT_NOT_DECLARED = 111;
export const PARSE_ERROR_COUNT = 112;

// ---- state ----

// Function flavours, for parameter and `super` rules.
const FL_NORMAL = 0;
const FL_METHOD = 1;
const FL_GETTER = 2;
const FL_SETTER = 3;
const FL_CONSTRUCTOR = 4;
const FL_DERIVED_CONSTRUCTOR = 5;

// The per-function context, replaced around every function body.
function newCtx(strict) {
  return {
    strict: strict,
    inFunction: false,
    isAsync: false,
    isGenerator: false,
    superProp: false,
    superCall: false,
    newTarget: false,
    // In a class field initializer or static block: no `arguments`.
    classInit: false,
    // In a class static block: `await` is reserved.
    staticBlock: false,
    labels: [],
    loopDepth: 0,
    switchDepth: 0,
  };
}

function copyCtx(c) {
  return {
    strict: c.strict,
    inFunction: c.inFunction,
    isAsync: c.isAsync,
    isGenerator: c.isGenerator,
    superProp: c.superProp,
    superCall: c.superCall,
    newTarget: c.newTarget,
    classInit: c.classInit,
    staticBlock: c.staticBlock,
    labels: c.labels,
    loopDepth: c.loopDepth,
    switchDepth: c.switchDepth,
  };
}

// A parser over `src` (code units). `isModule` picks the Module goal.
export function createParser(src, isModule) {
  return createParserWithLimits(src, isModule, MAX_SOURCE_BYTES, MAX_PARSE_DEPTH);
}

// The same with other limits (for tests).
export function createParserWithLimits(src, isModule, maxBytes, maxDepth) {
  const lx = createLexerWithLimit(src, isModule, maxBytes);
  const badRest = [];
  let b = 0;
  while (b < 256) {
    badRest.push([]);
    b = b + 1;
  }
  return {
    lx: lx,
    t: lx.tokens,
    src: src,
    isModule: isModule,
    // The current token: index, kind, punctuator id (-1 for another
    // kind), word id when it is an unescaped name (-1 otherwise), and
    // whether a line break comes before it.
    cur: -1,
    kind: TK_EOF,
    punct: -1,
    word: -1,
    nl: false,
    // The token after the current one, once looked at (-1: not yet).
    peeked: -1,
    prevEnd: 0,
    maxDepth: maxDepth,
    depth: 0,
    noIn: false,
    ctx: newCtx(isModule),
    scopeDepth: 0,
    // Declarations seen so far, for the early errors that need them.
    ee: newEarlyErrors(isModule),
    // A deferred error of the cover grammar, as { code, pos, line, col },
    // reported unless the expression turns into a pattern.
    coverErr: null,
    // Offsets of the last `yield` or `await` expression and of the last
    // `await` used as a name (-1: none), to reject them in parameters.
    lastYieldAwait: -1,
    lastAwaitIdent: -1,
    // Operator stack of parseUnary.
    prefix: [],
    // Offsets of spread elements followed by more elements, in 256
    // buckets by offset.
    badRest: badRest,
    // Results that do not fit a return value.
    becameStrict: false,
    coverTrailing: false,
    inBody: false,
    // The error record.
    err: PE_NONE,
    errPos: 0,
    errLine: 1,
    errCol: 1,
    // Token the message describes (-1: none).
    errTok: -1,
    // A name for the message (code units), or null.
    errName: null,
    // A punctuator or word id for the message, or the depth limit.
    errArg: 0,
  };
}

// ---- errors ----

function fail(p, code, pos, line, col) {
  if (p.err === PE_NONE) {
    p.err = code;
    p.errPos = pos;
    p.errLine = line;
    p.errCol = col;
  }
  return null;
}

// An error at the current token, which the message may describe.
function failCur(p, code) {
  const i = p.cur;
  if (p.err === PE_NONE) {
    p.errTok = i;
  }
  return fail(p, code, p.t.start[i], p.t.line[i], p.t.column[i]);
}

// An error at token i.
function failTok(p, code, i) {
  return fail(p, code, p.t.start[i], p.t.line[i], p.t.column[i]);
}

// An error at the start of node n.
function failNode(p, code, n) {
  return fail(p, code, n.start, n.line, n.col);
}

// The same, with a name for the message.
function failNodeName(p, code, n, name) {
  if (p.err === PE_NONE) {
    p.errName = name;
  }
  return fail(p, code, n.start, n.line, n.col);
}

function failTokName(p, code, i, name) {
  if (p.err === PE_NONE) {
    p.errName = name;
  }
  return failTok(p, code, i);
}

function failLex(p) {
  return fail(p, PE_LEX, 0, 1, 1);
}

// A deferred cover-grammar error at token i.
function coverError(code, p, i) {
  return { code: code, pos: p.t.start[i], line: p.t.line[i], col: p.t.column[i] };
}

function raiseCover(p, e) {
  return fail(p, e.code, e.pos, e.line, e.col);
}

// ---- tokens ----

function load(p, i) {
  const t = p.t;
  p.cur = i;
  p.kind = t.kind[i];
  p.nl = (t.flags[i] & F_NL) !== 0;
  if (p.kind === TK_PUNCT) {
    p.punct = t.aux[i];
  } else {
    p.punct = -1;
  }
  if (p.kind === TK_IDENT && (t.flags[i] & F_ESCAPED) === 0) {
    p.word = wordId(p.src, t.start[i], t.end[i]);
  } else {
    p.word = -1;
  }
}

// Move to the next token. False on a lexer error.
function bump(p) {
  let i = p.peeked;
  p.peeked = -1;
  if (i < 0) {
    i = nextToken(p.lx);
    if (i < 0) {
      failLex(p);
      return false;
    }
  }
  p.prevEnd = p.t.end[p.cur];
  load(p, i);
  return true;
}

// The token after the current one, without moving; -1 on a lexer error.
function peek(p) {
  if (p.peeked < 0) {
    const i = nextToken(p.lx);
    if (i < 0) {
      failLex(p);
      return -1;
    }
    p.peeked = i;
  }
  return p.peeked;
}

// Forget a looked-at token before the current one is read again.
function dropPeek(p) {
  if (p.peeked >= 0) {
    rewind(p.lx, p.cur + 1);
    p.peeked = -1;
  }
}

function tokPunct(p, i) {
  if (p.t.kind[i] === TK_PUNCT) {
    return p.t.aux[i];
  }
  return -1;
}

// Word id of token i when it is an unescaped name, or -1.
function tokWord(p, i) {
  const t = p.t;
  if (t.kind[i] === TK_IDENT && (t.flags[i] & F_ESCAPED) === 0) {
    return wordId(p.src, t.start[i], t.end[i]);
  }
  return -1;
}

function tokNl(p, i) {
  return (p.t.flags[i] & F_NL) !== 0;
}

function tokEscaped(p, i) {
  return (p.t.flags[i] & F_ESCAPED) !== 0;
}

function at(p, punct) {
  return p.punct === punct;
}

function atWord(p, w) {
  return p.word === w;
}

function atIdent(p) {
  return p.kind === TK_IDENT;
}

// Consume punctuator `punct` if it is next. Returns 1 when eaten, 0 when
// not, -1 on a lexer error.
function eat(p, punct) {
  if (p.punct === punct) {
    if (!bump(p)) {
      return -1;
    }
    return 1;
  }
  return 0;
}

function expect(p, punct) {
  if (p.punct === punct) {
    return bump(p);
  }
  if (p.err === PE_NONE) {
    p.errArg = punct;
  }
  failCur(p, PE_EXPECTED_PUNCT);
  return false;
}

function expectWord(p, w) {
  if (p.word === w) {
    return bump(p);
  }
  if (p.err === PE_NONE) {
    p.errArg = w;
  }
  failCur(p, PE_EXPECTED_WORD);
  return false;
}

function unexpected(p) {
  return failCur(p, PE_UNEXPECTED);
}

// End of a node that started at offset `start`.
function endFrom(p, start) {
  if (p.prevEnd > start) {
    return p.prevEnd;
  }
  return start;
}

// End of a node from token i to the last token consumed.
function endOf(p, i) {
  return endFrom(p, p.t.start[i]);
}

function maxEnd(a, b) {
  if (a.end > b.end) {
    return a.end;
  }
  return b.end;
}

// Automatic semicolon insertion (spec 12.10).
function semicolon(p) {
  if (p.punct === P_SEMICOLON) {
    return bump(p);
  }
  if (p.punct === P_RBRACE || p.kind === TK_EOF || p.nl) {
    return true;
  }
  if (p.err === PE_NONE) {
    p.errArg = P_SEMICOLON;
  }
  failCur(p, PE_EXPECTED_PUNCT);
  return false;
}

function enter(p) {
  p.depth = p.depth + 1;
  if (p.depth > p.maxDepth) {
    if (p.err === PE_NONE) {
      p.errArg = p.maxDepth;
    }
    failCur(p, PE_DEPTH);
    return false;
  }
  return true;
}

function leave(p) {
  if (p.depth > 0) {
    p.depth = p.depth - 1;
  }
}

function checkLegacy(p, i) {
  if ((p.t.flags[i] & F_LEGACY_OCTAL) !== 0 && p.ctx.strict) {
    failTok(p, PE_LEGACY_OCTAL, i);
    return false;
  }
  return true;
}

function awaitIsKeyword(p) {
  const c = p.ctx;
  return c.isAsync || (p.isModule && !c.inFunction && !c.staticBlock && !c.classInit);
}

function markBadRest(p, offset) {
  p.badRest[offset & 255].push(offset);
}

function isBadRest(p, offset) {
  const b = p.badRest[offset & 255];
  let i = 0;
  while (i < b.length) {
    if (b[i] === offset) {
      return true;
    }
    i = i + 1;
  }
  return false;
}

// ---- names ----

function unitsWord(units) {
  return wordId(units, 0, units.length);
}

// The decoded name of the current token (an ident or private name).
function curName(p) {
  return tokenName(p.t, p.src, p.cur);
}

// Word id of the decoded name of ident token i.
function nameWordOf(p, i) {
  if (!tokEscaped(p, i)) {
    return tokWord(p, i);
  }
  return unitsWord(tokenName(p.t, p.src, i));
}

// The rules for a name used as a reference (spec 13.1.1). `w` is the
// word id of `name`; the error sits at node or token place `n`.
function checkIdentRef(p, w, name, n) {
  if (isReservedWord(w)) {
    failNodeName(p, PE_RESERVED_WORD, n, name);
    return false;
  }
  if (p.ctx.strict && isStrictReservedWord(w)) {
    failNodeName(p, PE_STRICT_RESERVED, n, name);
    return false;
  }
  if (w === W_YIELD && p.ctx.isGenerator) {
    failNode(p, PE_YIELD_IN_GENERATOR, n);
    return false;
  }
  if (w === W_AWAIT && (awaitIsKeyword(p) || p.isModule || p.ctx.staticBlock)) {
    failNode(p, PE_AWAIT_NAME, n);
    return false;
  }
  return true;
}

// The same for a name being bound.
function checkBindingIdent(p, w, name, n) {
  if (!checkIdentRef(p, w, name, n)) {
    return false;
  }
  if (p.ctx.strict && (w === W_EVAL || w === W_ARGUMENTS)) {
    failNodeName(p, PE_CANT_BIND_STRICT, n, name);
    return false;
  }
  return true;
}

// A place record for a token, shaped like a node for the checks above.
function placeOf(p, i) {
  return { start: p.t.start[i], end: p.t.end[i], line: p.t.line[i], col: p.t.column[i] };
}

// Names bound by pattern `pat`, appended to `out` as Pat.Ident nodes.
function boundNames(pat, out) {
  const k = pat.kind;
  if (k === PT_IDENT) {
    out.push(pat);
  } else if (k === PT_ARRAY) {
    let i = 0;
    while (i < pat.items.length) {
      if (pat.items[i] !== null) {
        boundNames(pat.items[i], out);
      }
      i = i + 1;
    }
  } else if (k === PT_OBJECT) {
    let i = 0;
    while (i < pat.props.length) {
      const pp = pat.props[i];
      if (pp.kind === PP_PROP) {
        boundNames(pp.value, out);
      } else {
        boundNames(pp.arg, out);
      }
      i = i + 1;
    }
  } else if (k === PT_ASSIGN) {
    boundNames(pat.target, out);
  } else if (k === PT_REST) {
    boundNames(pat.arg, out);
  }
}

function boundNamesOfList(pats) {
  const out = [];
  let i = 0;
  while (i < pats.length) {
    boundNames(pats[i], out);
    i = i + 1;
  }
  return out;
}

// After a body turned strict: parameter (or function) names that strict
// code can't bind.
function checkStrictNames(p, names) {
  let i = 0;
  while (i < names.length) {
    const n = names[i];
    const w = unitsWord(n.name);
    if (w === W_EVAL || w === W_ARGUMENTS) {
      failNodeName(p, PE_CANT_BIND_STRICT, n, n.name);
      return false;
    }
    if (isStrictReservedWord(w)) {
      failNodeName(p, PE_STRICT_RESERVED, n, n.name);
      return false;
    }
    i = i + 1;
  }
  return true;
}

function checkUnique(p, names) {
  const seen = newNameSet();
  let i = 0;
  while (i < names.length) {
    if (!nameSetAdd(seen, names[i].name)) {
      failNodeName(p, PE_DUPLICATE_PARAM, names[i], names[i].name);
      return false;
    }
    i = i + 1;
  }
  return true;
}

function allIdents(pats) {
  let i = 0;
  while (i < pats.length) {
    if (pats[i].kind !== PT_IDENT) {
      return false;
    }
    i = i + 1;
  }
  return true;
}

// The parenthesized expression's innermost content.
function unparen(e) {
  let x = e;
  while (x.kind === E_PAREN) {
    x = x.expr;
  }
  return x;
}

function isArrow(e) {
  return e.kind === E_ARROW;
}

// Word id of a property key's name (identifier or string), or -1.
function keyWord(key) {
  if (key.kind === PK_IDENT) {
    return unitsWord(key.ident.name);
  }
  if (key.kind === PK_STR) {
    return unitsWord(key.value);
  }
  return -1;
}

// Whether token i can start a property key.
function startsKey(p, i, allowPrivate) {
  const k = p.t.kind[i];
  if (k === TK_IDENT || k === TK_STRING || k === TK_NUMBER || k === TK_BIGINT) {
    return true;
  }
  if (k === TK_PRIVATE) {
    return allowPrivate;
  }
  return tokPunct(p, i) === P_LBRACKET;
}

// Ident node from token i.
function identOf(p, i) {
  const t = p.t;
  return makeIdent(tokenName(t, p.src, i), t.start[i], t.end[i], t.line[i], t.column[i]);
}

// Pat.Ident with the place of node or token record n.
function patIdentAt(name, n) {
  return makePatIdent(name, n.start, n.end, n.line, n.col);
}

// Scopes: `isFunction` for scripts, modules, functions, arrows, field
// initializers and static blocks; block scopes otherwise.
function pushScope(p, isFunction) {
  p.scopeDepth = p.scopeDepth + 1;
  enterScope(p.ee, isFunction);
}

function popScope(p) {
  p.scopeDepth = p.scopeDepth - 1;
  exitScope(p.ee);
}

// Declaration helpers: false with the error set.
function lexicalName(p, id) {
  if (!declareLexical(p.ee, id.name)) {
    failNodeName(p, PE_REDECLARED, id, id.name);
    return false;
  }
  return true;
}

function varName(p, id) {
  if (!declareVar(p.ee, id.name)) {
    failNodeName(p, PE_REDECLARED, id, id.name);
    return false;
  }
  return true;
}

function exportName(p, m) {
  const name = moduleNameText(m);
  if (!addExport(p.ee, name)) {
    failNodeName(p, PE_EXPORTED_TWICE, moduleNameNode(m), name);
    return false;
  }
  return true;
}

function exportIdent(p, id) {
  if (!addExport(p.ee, id.name)) {
    failNodeName(p, PE_EXPORTED_TWICE, id, id.name);
    return false;
  }
  return true;
}

// ---- program and statement lists ----

// Parse the whole source. Returns the Program node, or null with the
// error in the state.
export function parseProgram(p) {
  if (p.lx.error !== E_NONE) {
    return failLex(p);
  }
  const first = nextToken(p.lx);
  if (first < 0) {
    return failLex(p);
  }
  load(p, first);
  pushScope(p, true);
  const body = parseBody(p, false);
  if (body === null) {
    return null;
  }
  if (p.isModule) {
    const missing = checkExportLocals(p.ee);
    if (missing !== null) {
      return failNodeName(p, PE_EXPORT_NOT_DECLARED, missing, missing.name);
    }
  }
  return makeProgram(p.isModule, p.ctx.strict, body, 0, p.src.length, 1, 1);
}

const USE_STRICT = [0x75, 0x73, 0x65, 0x20, 0x73, 0x74, 0x72, 0x69, 0x63, 0x74];

// Statements up to `}` (when `braced`, which is left for the caller) or
// the end, with a directive prologue first. Sets p.becameStrict when a
// "use strict" directive switched strict mode on.
function parseBody(p, braced) {
  const t = p.t;
  const body = [];
  let prologue = true;
  let octal = -1;
  let becameStrict = false;
  while (true) {
    if (braced && p.punct === P_RBRACE) {
      break;
    }
    if (p.kind === TK_EOF) {
      if (braced) {
        return unexpected(p);
      }
      break;
    }
    if (prologue && p.kind === TK_STRING) {
      const tok = p.cur;
      const stmt = parseStatementListItem(p);
      if (stmt === null) {
        return null;
      }
      let isDirective = false;
      if (
        stmt.kind === S_EXPR &&
        stmt.expr.kind === E_STR &&
        stmt.expr.start === t.start[tok] &&
        stmt.expr.end === t.end[tok]
      ) {
        isDirective = true;
        if ((t.flags[tok] & F_LEGACY_OCTAL) !== 0 && octal < 0) {
          octal = tok;
        }
        if (rangeIs(p.src, t.start[tok] + 1, t.end[tok] - 1, USE_STRICT)) {
          if (!p.ctx.strict) {
            p.ctx.strict = true;
            becameStrict = true;
          }
          if (octal >= 0) {
            return failTok(p, PE_OCTAL_DIRECTIVE, octal);
          }
        }
        stmt.directive = sliceUnits(p.src, t.start[tok] + 1, t.end[tok] - 1);
      }
      if (!isDirective) {
        prologue = false;
      }
      body.push(stmt);
    } else {
      prologue = false;
      const s = parseStatementListItem(p);
      if (s === null) {
        return null;
      }
      body.push(s);
    }
  }
  p.becameStrict = becameStrict;
  return body;
}

function parseStatementListItem(p) {
  if (!enter(p)) {
    return null;
  }
  let r = null;
  if (p.kind !== TK_IDENT) {
    r = statementInner(p);
  } else {
    r = declarationOrStatement(p);
  }
  leave(p);
  return r;
}

function declarationOrStatement(p) {
  const st = p.cur;
  const w = p.word;
  if (w === W_FUNCTION) {
    return parseFunctionDeclaration(p, st, false);
  }
  if (w === W_ASYNC) {
    const q = peek(p);
    if (q < 0) {
      return null;
    }
    if (tokWord(p, q) === W_FUNCTION && !tokNl(p, q)) {
      if (!bump(p)) {
        return null;
      }
      return parseFunctionDeclaration(p, st, true);
    }
  }
  if (w === W_CLASS) {
    const c = parseClass(p, true, false);
    if (c === null || !lexicalName(p, c.id)) {
      return null;
    }
    return makeStmtClass(c, p.t.start[st], endOf(p, st), p.t.line[st], p.t.column[st]);
  }
  let lexical = w === W_CONST;
  if (w === W_LET) {
    const s = letStartsDeclaration(p);
    if (s < 0) {
      return null;
    }
    lexical = s === 1;
  }
  if (lexical) {
    let kind = VK_LET;
    if (w === W_CONST) {
      kind = VK_CONST;
    }
    const d = parseVarDecl(p, kind, false);
    if (d === null || !semicolon(p)) {
      return null;
    }
    return makeStmtVar(d, p.t.start[st], endOf(p, st), p.t.line[st], p.t.column[st]);
  }
  if (w === W_IMPORT) {
    const q = peek(p);
    if (q < 0) {
      return null;
    }
    const qp = tokPunct(p, q);
    if (qp !== P_LPAREN && qp !== P_DOT) {
      return parseImport(p, st);
    }
  }
  if (w === W_EXPORT) {
    return parseExport(p, st);
  }
  return statementInner(p);
}

// 1 when `let` here starts a declaration, 0 when not, -1 on error.
function letStartsDeclaration(p) {
  const q = peek(p);
  if (q < 0) {
    return -1;
  }
  const qp = tokPunct(p, q);
  if (qp === P_LBRACKET || qp === P_LBRACE) {
    return 1;
  }
  if (p.t.kind[q] === TK_IDENT && (tokEscaped(p, q) || !isReservedWord(tokWord(p, q)))) {
    return 1;
  }
  return 0;
}

function parseStatement(p) {
  if (!enter(p)) {
    return null;
  }
  const r = statementInner(p);
  leave(p);
  return r;
}

// A statement where only a statement (not a declaration) may stand.
// `p.inBody` is set when it is the body of `if`, `with` or a loop, where
// a labelled function declaration is not allowed either.
function statementInner(p) {
  const inBody = p.inBody;
  p.inBody = false;
  const st = p.cur;
  const t = p.t;
  if (p.punct === P_LBRACE) {
    const body = parseBlock(p, null, false);
    if (body === null) {
      return null;
    }
    return makeStmtBlock(body, t.start[st], endOf(p, st), t.line[st], t.column[st]);
  }
  if (p.word < 0) {
    if (p.punct === P_SEMICOLON) {
      if (!bump(p)) {
        return null;
      }
      return makeStmtEmpty(t.start[st], t.end[st], t.line[st], t.column[st]);
    }
    return parseExpressionOrLabeled(p, st, inBody);
  }
  const w = p.word;
  if (w === W_IF) {
    return parseIf(p, st);
  }
  if (w === W_FOR) {
    return parseFor(p, st);
  }
  if (w === W_WHILE) {
    return parseWhile(p, st);
  }
  if (w === W_DO) {
    return parseDo(p, st);
  }
  if (w === W_TRY) {
    return parseTry(p, st);
  }
  if (w === W_SWITCH) {
    return parseSwitch(p, st);
  }
  if (w === W_WITH) {
    return parseWith(p, st);
  }
  return statementOther(p, st, inBody);
}

function parseWhile(p, st) {
  if (!bump(p) || !expect(p, P_LPAREN)) {
    return null;
  }
  const test = parseExpressionIn(p);
  if (test === null || !expect(p, P_RPAREN)) {
    return null;
  }
  const body = parseLoopBody(p);
  if (body === null) {
    return null;
  }
  const t = p.t;
  return makeStmtWhile(test, body, t.start[st], endOf(p, st), t.line[st], t.column[st]);
}

function parseDo(p, st) {
  if (!bump(p)) {
    return null;
  }
  const body = parseLoopBody(p);
  if (body === null || !expectWord(p, W_WHILE) || !expect(p, P_LPAREN)) {
    return null;
  }
  const test = parseExpressionIn(p);
  if (test === null || !expect(p, P_RPAREN) || eat(p, P_SEMICOLON) < 0) {
    return null;
  }
  const t = p.t;
  return makeStmtDoWhile(body, test, t.start[st], endOf(p, st), t.line[st], t.column[st]);
}

function parseWith(p, st) {
  if (p.ctx.strict) {
    return failTok(p, PE_WITH_STRICT, st);
  }
  if (!bump(p) || !expect(p, P_LPAREN)) {
    return null;
  }
  const object = parseExpressionIn(p);
  if (object === null || !expect(p, P_RPAREN)) {
    return null;
  }
  const body = parseBodyStatement(p);
  if (body === null) {
    return null;
  }
  const t = p.t;
  return makeStmtWith(object, body, t.start[st], endOf(p, st), t.line[st], t.column[st]);
}

// Statements that start with a word other than the nesting keywords.
function statementOther(p, st, inBody) {
  const t = p.t;
  const w = p.word;
  if (w === W_VAR) {
    const d = parseVarDecl(p, VK_VAR, false);
    if (d === null || !semicolon(p)) {
      return null;
    }
    return makeStmtVar(d, t.start[st], endOf(p, st), t.line[st], t.column[st]);
  }
  if (w === W_CONTINUE || w === W_BREAK) {
    return parseBreakContinue(p, st, w === W_BREAK);
  }
  if (w === W_RETURN) {
    if (!p.ctx.inFunction) {
      return failTok(p, PE_RETURN_OUTSIDE, st);
    }
    if (!bump(p)) {
      return null;
    }
    let arg = null;
    if (!(p.punct === P_SEMICOLON || p.punct === P_RBRACE || p.kind === TK_EOF || p.nl)) {
      arg = parseExpressionIn(p);
      if (arg === null) {
        return null;
      }
    }
    if (!semicolon(p)) {
      return null;
    }
    return makeStmtReturn(arg, t.start[st], endOf(p, st), t.line[st], t.column[st]);
  }
  if (w === W_THROW) {
    if (!bump(p)) {
      return null;
    }
    if (p.nl) {
      return failTok(p, PE_THROW_NEWLINE, p.cur);
    }
    const arg = parseExpressionIn(p);
    if (arg === null || !semicolon(p)) {
      return null;
    }
    return makeStmtThrow(arg, t.start[st], endOf(p, st), t.line[st], t.column[st]);
  }
  if (w === W_DEBUGGER) {
    if (!bump(p) || !semicolon(p)) {
      return null;
    }
    return makeStmtDebugger(t.start[st], endOf(p, st), t.line[st], t.column[st]);
  }
  if (w === W_FUNCTION) {
    return failTok(p, PE_FUNCTION_DECL_HERE, st);
  }
  if (w === W_CLASS || w === W_CONST) {
    return failTok(p, PE_DECL_HERE, st);
  }
  if (w === W_LET) {
    const q = peek(p);
    if (q < 0) {
      return null;
    }
    const qp = tokPunct(p, q);
    if (qp === P_LBRACKET) {
      return failTok(p, PE_LET_BRACKET, st);
    }
    if ((p.t.kind[q] === TK_IDENT || qp === P_LBRACE) && !tokNl(p, q)) {
      return failTok(p, PE_LEXICAL_HERE, st);
    }
    return parseExpressionOrLabeled(p, st, inBody);
  }
  if (w === W_ASYNC) {
    const q = peek(p);
    if (q < 0) {
      return null;
    }
    if (tokWord(p, q) === W_FUNCTION && !tokNl(p, q)) {
      return failTok(p, PE_ASYNC_FUNCTION_HERE, st);
    }
  }
  return parseExpressionOrLabeled(p, st, inBody);
}

function parseExpressionOrLabeled(p, st, inBody) {
  if (p.kind === TK_IDENT) {
    const q = peek(p);
    if (q < 0) {
      return null;
    }
    if (tokPunct(p, q) === P_COLON) {
      return parseLabeled(p, st, inBody);
    }
  }
  const e = parseExpression(p);
  if (e === null || !semicolon(p)) {
    return null;
  }
  const t = p.t;
  return makeStmtExpr(e, null, t.start[st], endOf(p, st), t.line[st], t.column[st]);
}

function findLabel(p, name) {
  const labels = p.ctx.labels;
  let i = 0;
  while (i < labels.length) {
    if (sameUnits(labels[i].name, name)) {
      return labels[i];
    }
    i = i + 1;
  }
  return null;
}

function parseLabeled(p, st, inBody) {
  const t = p.t;
  const names = [];
  while (p.kind === TK_IDENT) {
    const q = peek(p);
    if (q < 0) {
      return null;
    }
    if (tokPunct(p, q) !== P_COLON) {
      break;
    }
    const tok = p.cur;
    const name = curName(p);
    if (!checkIdentRef(p, nameWordOf(p, tok), name, placeOf(p, tok))) {
      return null;
    }
    if (findLabel(p, name) !== null) {
      return failTokName(p, PE_LABEL_IN_USE, tok, name);
    }
    if (!bump(p) || !bump(p)) {
      return null;
    }
    p.ctx.labels.push({ name: name, isLoop: false });
    names.push(identOf(p, tok));
  }
  const labels = p.ctx.labels;
  if (p.word === W_FOR || p.word === W_WHILE || p.word === W_DO) {
    let i = labels.length - names.length;
    while (i < labels.length) {
      labels[i].isLoop = true;
      i = i + 1;
    }
  }
  let body = null;
  if (p.word === W_FUNCTION) {
    if (p.ctx.strict) {
      return failTok(p, PE_LABELED_FUNCTION_STRICT, p.cur);
    }
    if (inBody) {
      // IsLabelledFunction: the body of `if`, `with` or a loop can't be
      // a labelled function declaration (the reference allows it; see
      // tests/DIFFERENCES.md, "labelled function as
      // a statement body").
      return failTok(p, PE_FUNCTION_DECL_HERE, p.cur);
    }
    const fst = p.cur;
    body = parseFunctionDeclaration(p, fst, false);
    if (body === null) {
      return null;
    }
    if (body.func.isGenerator || body.func.isAsync) {
      return failTok(p, PE_ONLY_PLAIN_LABELED, fst);
    }
  } else {
    body = parseStatement(p);
    if (body === null) {
      return null;
    }
  }
  let k = 0;
  while (k < names.length) {
    labels.pop();
    k = k + 1;
  }
  let stmt = body;
  let i = names.length - 1;
  while (i >= 0) {
    stmt = makeStmtLabeled(names[i], stmt, t.start[st], endOf(p, st), t.line[st], t.column[st]);
    i = i - 1;
  }
  return stmt;
}

function parseBreakContinue(p, st, isBreak) {
  const t = p.t;
  if (!bump(p)) {
    return null;
  }
  let label = null;
  if (!p.nl && p.kind === TK_IDENT) {
    const name = curName(p);
    const l = findLabel(p, name);
    if (l === null) {
      return failTokName(p, PE_LABEL_NOT_DEFINED, p.cur, name);
    }
    if (!isBreak && !l.isLoop) {
      return failTokName(p, PE_CONTINUE_NOT_LOOP, p.cur, name);
    }
    label = identOf(p, p.cur);
    if (!bump(p)) {
      return null;
    }
  }
  if (label === null) {
    if (isBreak && p.ctx.loopDepth === 0 && p.ctx.switchDepth === 0) {
      return failTok(p, PE_BREAK_OUTSIDE, st);
    }
    if (!isBreak && p.ctx.loopDepth === 0) {
      return failTok(p, PE_CONTINUE_OUTSIDE, st);
    }
  }
  if (!semicolon(p)) {
    return null;
  }
  if (isBreak) {
    return makeStmtBreak(label, t.start[st], endOf(p, st), t.line[st], t.column[st]);
  }
  return makeStmtContinue(label, t.start[st], endOf(p, st), t.line[st], t.column[st]);
}

// `{ ... }`; returns the statements.
// A block; `catchNames` (a name set, or null) and `catchSimple` describe
// the parameter when it is the body of a `catch`.
function parseBlock(p, catchNames, catchSimple) {
  if (!expect(p, P_LBRACE)) {
    return null;
  }
  pushScope(p, false);
  if (catchNames !== null) {
    setCatchParams(p.ee, catchNames, catchSimple);
  }
  const body = [];
  while (p.punct !== P_RBRACE) {
    if (p.kind === TK_EOF) {
      return unexpected(p);
    }
    const s = parseStatementListItem(p);
    if (s === null) {
      return null;
    }
    body.push(s);
  }
  if (!bump(p)) {
    return null;
  }
  popScope(p);
  return body;
}

// The body of `if`, `with` or a loop.
function parseBodyStatement(p) {
  p.inBody = true;
  return parseStatement(p);
}

function parseLoopBody(p) {
  p.ctx.loopDepth = p.ctx.loopDepth + 1;
  const body = parseBodyStatement(p);
  p.ctx.loopDepth = p.ctx.loopDepth - 1;
  return body;
}

function parseIf(p, st) {
  if (!bump(p) || !expect(p, P_LPAREN)) {
    return null;
  }
  const test = parseExpressionIn(p);
  if (test === null || !expect(p, P_RPAREN)) {
    return null;
  }
  const cons = parseIfBranch(p);
  if (cons === null) {
    return null;
  }
  let alt = null;
  if (p.word === W_ELSE) {
    if (!bump(p)) {
      return null;
    }
    alt = parseIfBranch(p);
    if (alt === null) {
      return null;
    }
  }
  const t = p.t;
  return makeStmtIf(test, cons, alt, t.start[st], endOf(p, st), t.line[st], t.column[st]);
}

function parseIfBranch(p) {
  if (p.word === W_FUNCTION && !p.ctx.strict) {
    // Annex B.3.3: a plain function declaration as an `if` branch, as if
    // it sat in a block of its own.
    const fst = p.cur;
    pushScope(p, false);
    const f = parseFunctionDeclaration(p, fst, false);
    popScope(p);
    if (f === null) {
      return null;
    }
    if (f.func.isGenerator) {
      return failTok(p, PE_GENERATOR_DECL_HERE, fst);
    }
    return f;
  }
  return parseBodyStatement(p);
}

function parseSwitch(p, st) {
  const t = p.t;
  if (!bump(p) || !expect(p, P_LPAREN)) {
    return null;
  }
  const disc = parseExpressionIn(p);
  if (disc === null || !expect(p, P_RPAREN) || !expect(p, P_LBRACE)) {
    return null;
  }
  pushScope(p, false);
  p.ctx.switchDepth = p.ctx.switchDepth + 1;
  const cases = [];
  let seenDefault = false;
  while (p.punct !== P_RBRACE) {
    const cst = p.cur;
    let test = null;
    if (p.word === W_CASE) {
      if (!bump(p)) {
        return null;
      }
      test = parseExpressionIn(p);
      if (test === null) {
        return null;
      }
    } else if (p.word === W_DEFAULT) {
      if (seenDefault) {
        return failTok(p, PE_SWITCH_DEFAULT, cst);
      }
      seenDefault = true;
      if (!bump(p)) {
        return null;
      }
    } else {
      return unexpected(p);
    }
    if (!expect(p, P_COLON)) {
      return null;
    }
    const body = [];
    while (p.punct !== P_RBRACE && p.word !== W_CASE && p.word !== W_DEFAULT) {
      if (p.kind === TK_EOF) {
        return unexpected(p);
      }
      const s = parseStatementListItem(p);
      if (s === null) {
        return null;
      }
      body.push(s);
    }
    cases.push(makeSwitchCase(test, body, t.start[cst], endOf(p, cst), t.line[cst], t.column[cst]));
  }
  if (!bump(p)) {
    return null;
  }
  p.ctx.switchDepth = p.ctx.switchDepth - 1;
  popScope(p);
  return makeStmtSwitch(disc, cases, t.start[st], endOf(p, st), t.line[st], t.column[st]);
}

function parseTry(p, st) {
  const t = p.t;
  if (!bump(p)) {
    return null;
  }
  const block = parseBlock(p, null, false);
  if (block === null) {
    return null;
  }
  let handler = null;
  if (p.word === W_CATCH) {
    const cst = p.cur;
    if (!bump(p)) {
      return null;
    }
    let param = null;
    let names = null;
    const e = eat(p, P_LPAREN);
    if (e < 0) {
      return null;
    }
    if (e === 1) {
      param = parseBindingTarget(p);
      if (param === null) {
        return null;
      }
      const bound = [];
      boundNames(param, bound);
      names = newNameSet();
      let i = 0;
      while (i < bound.length) {
        if (!nameSetAdd(names, bound[i].name)) {
          return failNodeName(p, PE_DUPLICATE_CATCH_PARAM, bound[i], bound[i].name);
        }
        i = i + 1;
      }
      if (!expect(p, P_RPAREN)) {
        return null;
      }
    }
    const body = parseBlock(p, names, param !== null && param.kind === PT_IDENT);
    if (body === null) {
      return null;
    }
    handler = makeCatch(param, body, t.start[cst], endOf(p, cst), t.line[cst], t.column[cst]);
  }
  let finalizer = null;
  if (p.word === W_FINALLY) {
    if (!bump(p)) {
      return null;
    }
    finalizer = parseBlock(p, null, false);
    if (finalizer === null) {
      return null;
    }
  }
  if (handler === null && finalizer === null) {
    return failTok(p, PE_TRY_NEEDS, st);
  }
  return makeStmtTry(block, handler, finalizer, t.start[st], endOf(p, st), t.line[st], t.column[st]);
}

// `var`, `let` or `const` and the declarators. `forHead`: in a for head,
// where `in` or `of` may stand in for an initializer.
function parseVarDecl(p, kind, forHead) {
  const t = p.t;
  const st = p.cur;
  if (!bump(p)) {
    return null;
  }
  const decls = [];
  while (true) {
    const dst = p.cur;
    const target = parseBindingTarget(p);
    if (target === null) {
      return null;
    }
    const names = [];
    boundNames(target, names);
    let i = 0;
    while (i < names.length) {
      if (kind === VK_VAR) {
        if (!varName(p, names[i])) {
          return null;
        }
      } else {
        if (unitsWord(names[i].name) === W_LET) {
          return failNode(p, PE_LET_LEXICAL, names[i]);
        }
        if (!lexicalName(p, names[i])) {
          return null;
        }
      }
      i = i + 1;
    }
    let init = null;
    const e = eat(p, P_EQ);
    if (e < 0) {
      return null;
    }
    if (e === 1) {
      init = parseAssign(p);
      if (init === null) {
        return null;
      }
    } else if (!forHead || !(p.word === W_IN || p.word === W_OF)) {
      if (kind === VK_CONST) {
        return failTok(p, PE_CONST_INIT, dst);
      }
      if (target.kind !== PT_IDENT) {
        return failTok(p, PE_DESTRUCTURING_INIT, dst);
      }
    }
    decls.push(makeVarDeclarator(target, init, t.start[dst], endOf(p, dst), t.line[dst], t.column[dst]));
    const c = eat(p, P_COMMA);
    if (c < 0) {
      return null;
    }
    if (c === 0) {
      break;
    }
  }
  return makeVarDecl(kind, decls, t.start[st], endOf(p, st), t.line[st], t.column[st]);
}

function parseFor(p, st) {
  if (!bump(p)) {
    return null;
  }
  let isAwait = false;
  if (p.word === W_AWAIT) {
    if (!awaitIsKeyword(p)) {
      return failTok(p, PE_FOR_AWAIT_ASYNC, p.cur);
    }
    if (!bump(p)) {
      return null;
    }
    isAwait = true;
  }
  if (!expect(p, P_LPAREN)) {
    return null;
  }
  pushScope(p, false);
  const r = parseForRest(p, st, isAwait);
  popScope(p);
  return r;
}

function parseForRest(p, st, isAwait) {
  const t = p.t;
  let decl = null;
  let initExpr = null;
  const startsWithLet = p.word === W_LET;
  const startsWithAsync = p.word === W_ASYNC;
  let isDecl = p.word === W_VAR || p.word === W_CONST;
  if (p.word === W_LET) {
    const q = peek(p);
    if (q < 0) {
      return null;
    }
    const qp = tokPunct(p, q);
    if (qp === P_LBRACKET || qp === P_LBRACE) {
      isDecl = true;
    } else if (p.t.kind[q] === TK_IDENT) {
      isDecl = tokEscaped(p, q) || tokWord(p, q) !== W_IN;
    }
  }
  if (p.punct === P_SEMICOLON) {
    // No initializer.
  } else if (isDecl) {
    let kind = VK_LET;
    if (p.word === W_VAR) {
      kind = VK_VAR;
    } else if (p.word === W_CONST) {
      kind = VK_CONST;
    }
    const savedIn = p.noIn;
    p.noIn = true;
    decl = parseVarDecl(p, kind, true);
    p.noIn = savedIn;
    if (decl === null) {
      return null;
    }
  } else {
    const savedIn = p.noIn;
    p.noIn = true;
    const savedCover = p.coverErr;
    p.coverErr = null;
    const first = parseAssignInner(p, true);
    p.noIn = savedIn;
    if (first === null) {
      return null;
    }
    if (p.punct === P_COMMA) {
      if (p.coverErr !== null) {
        return raiseCover(p, p.coverErr);
      }
      const items = [first];
      const savedIn2 = p.noIn;
      p.noIn = true;
      while (true) {
        const c = eat(p, P_COMMA);
        if (c < 0) {
          return null;
        }
        if (c === 0) {
          break;
        }
        const x = parseAssign(p);
        if (x === null) {
          return null;
        }
        items.push(x);
      }
      p.noIn = savedIn2;
      const last = items[items.length - 1];
      initExpr = makeExprSeq(items, first.start, maxEnd(first, last), first.line, first.col);
    } else {
      initExpr = first;
    }
    if (!(p.word === W_OF || p.word === W_IN)) {
      if (p.coverErr !== null) {
        return raiseCover(p, p.coverErr);
      }
    }
    p.coverErr = savedCover;
  }
  const isOf = p.word === W_OF;
  const isIn = p.word === W_IN;
  if (isOf || isIn) {
    if (isIn && isAwait) {
      return failTok(p, PE_FOR_AWAIT_OF, p.cur);
    }
    let head = null;
    if (decl !== null) {
      if (decl.decls.length !== 1) {
        return failNode(p, PE_FOR_ONE_BINDING, decl);
      }
      const d0 = decl.decls[0];
      if (d0.init !== null) {
        const annexB = isIn && !p.ctx.strict && decl.varKind === VK_VAR && d0.target.kind === PT_IDENT;
        if (!annexB) {
          return failNode(p, PE_FOR_INIT, d0.init);
        }
      }
      head = makeForHeadVar(decl, decl.start, decl.end, decl.line, decl.col);
    } else {
      if (initExpr === null) {
        return unexpected(p);
      }
      const e = initExpr;
      if (isOf && startsWithLet) {
        return failNode(p, PE_FOR_OF_LET, e);
      }
      if (isOf && startsWithAsync && !isAwait && e.kind === E_IDENT && unitsWord(e.name) === W_ASYNC) {
        return failNode(p, PE_FOR_ASYNC_OF, e);
      }
      const pat = exprToTarget(p, e);
      if (pat === null) {
        return null;
      }
      p.coverErr = null;
      head = makeForHeadPat(pat, pat.start, pat.end, pat.line, pat.col);
    }
    if (!bump(p)) {
      return null;
    }
    let right = null;
    if (isOf) {
      right = parseAssignIn(p);
    } else {
      right = parseExpressionIn(p);
    }
    if (right === null || !expect(p, P_RPAREN)) {
      return null;
    }
    const body = parseLoopBody(p);
    if (body === null) {
      return null;
    }
    if (isOf) {
      return makeStmtForOf(head, right, body, isAwait, t.start[st], endOf(p, st), t.line[st], t.column[st]);
    }
    return makeStmtForIn(head, right, body, t.start[st], endOf(p, st), t.line[st], t.column[st]);
  }
  if (isAwait) {
    return failTok(p, PE_FOR_AWAIT_OF, p.cur);
  }
  let init = null;
  if (decl !== null) {
    let i = 0;
    while (i < decl.decls.length) {
      const v = decl.decls[i];
      if (v.init === null) {
        if (decl.varKind === VK_CONST) {
          return failNode(p, PE_CONST_INIT, v);
        }
        if (v.target.kind !== PT_IDENT) {
          return failNode(p, PE_DESTRUCTURING_INIT, v);
        }
      }
      i = i + 1;
    }
    init = makeForInitVar(decl, decl.start, decl.end, decl.line, decl.col);
  } else if (initExpr !== null) {
    init = makeForInitExpr(initExpr, initExpr.start, initExpr.end, initExpr.line, initExpr.col);
  }
  if (!expect(p, P_SEMICOLON)) {
    return null;
  }
  let test = null;
  if (p.punct !== P_SEMICOLON) {
    test = parseExpressionIn(p);
    if (test === null) {
      return null;
    }
  }
  if (!expect(p, P_SEMICOLON)) {
    return null;
  }
  let update = null;
  if (p.punct !== P_RPAREN) {
    update = parseExpressionIn(p);
    if (update === null) {
      return null;
    }
  }
  if (!expect(p, P_RPAREN)) {
    return null;
  }
  const body = parseLoopBody(p);
  if (body === null) {
    return null;
  }
  return makeStmtFor(init, test, update, body, t.start[st], endOf(p, st), t.line[st], t.column[st]);
}

// ---- functions ----

function parseFunctionDeclaration(p, st, isAsync) {
  const f = parseFunction(p, st, isAsync, true, false);
  if (f === null) {
    return null;
  }
  const t = p.t;
  return makeStmtFunction(f, t.start[st], endOf(p, st), t.line[st], t.column[st]);
}

// From the `function` keyword on (`async` already consumed). `st` is the
// token the function starts at.
function parseFunction(p, st, isAsync, isDecl, nameOptional) {
  if (!expectWord(p, W_FUNCTION)) {
    return null;
  }
  const g = eat(p, P_STAR);
  if (g < 0) {
    return null;
  }
  const isGenerator = g === 1;
  let id = null;
  if (p.kind === TK_IDENT) {
    const i = p.cur;
    const name = curName(p);
    const w = nameWordOf(p, i);
    const place = placeOf(p, i);
    if (isDecl) {
      if (!checkBindingIdent(p, w, name, place)) {
        return null;
      }
    } else {
      // A function expression's name follows its own generator and async
      // rules.
      const saved = p.ctx;
      p.ctx = copyCtx(saved);
      p.ctx.isGenerator = isGenerator;
      p.ctx.isAsync = isAsync;
      p.ctx.inFunction = true;
      const ok = checkBindingIdent(p, w, name, place);
      p.ctx = saved;
      if (!ok) {
        return null;
      }
    }
    if (!bump(p)) {
      return null;
    }
    id = makeIdent(name, place.start, place.end, place.line, place.col);
    if (isDecl && !declareFunction(p.ee, name, !isAsync && !isGenerator, p.ctx.strict)) {
      return failNodeName(p, PE_REDECLARED, id, name);
    }
  } else if (isDecl && !nameOptional) {
    return failCur(p, PE_FUNCTION_NEEDS_NAME);
  }
  return parseFunctionRest(p, st, id, isAsync, isGenerator, FL_NORMAL);
}

// Parameters and body, in a fresh function context.
function parseFunctionRest(p, st, id, isAsync, isGenerator, flavor) {
  const outer = p.ctx;
  const savedYieldAwait = p.lastYieldAwait;
  const savedAwaitIdent = p.lastAwaitIdent;
  const savedIn = p.noIn;
  p.noIn = false;
  const c = newCtx(outer.strict);
  c.inFunction = true;
  c.isAsync = isAsync;
  c.isGenerator = isGenerator;
  c.superProp = flavor !== FL_NORMAL;
  c.superCall = flavor === FL_DERIVED_CONSTRUCTOR;
  c.newTarget = true;
  p.ctx = c;
  pushScope(p, true);
  const r = functionRestInner(p, st, id, isAsync, isGenerator, flavor);
  popScope(p);
  p.ctx = outer;
  p.noIn = savedIn;
  p.lastYieldAwait = savedYieldAwait;
  p.lastAwaitIdent = savedAwaitIdent;
  return r;
}

function functionRestInner(p, st, id, isAsync, isGenerator, flavor) {
  const t = p.t;
  const pst = p.cur;
  if (!expect(p, P_LPAREN)) {
    return null;
  }
  const params = parseFormalParams(p);
  if (params === null) {
    return null;
  }
  if (p.lastYieldAwait >= 0 && p.lastYieldAwait >= t.start[pst]) {
    return failTok(p, PE_YIELD_AWAIT_PARAMS, pst);
  }
  if (flavor === FL_GETTER && params.length !== 0) {
    return failTok(p, PE_GETTER_PARAMS, pst);
  }
  if (flavor === FL_SETTER && (params.length !== 1 || params[0].kind === PT_REST)) {
    return failTok(p, PE_SETTER_PARAMS, pst);
  }
  const simple = allIdents(params);
  const names = boundNamesOfList(params);
  setParams(p.ee, names);
  if (!expect(p, P_LBRACE)) {
    return null;
  }
  const body = parseBody(p, true);
  if (body === null) {
    return null;
  }
  const becameStrict = p.becameStrict;
  if (!expect(p, P_RBRACE)) {
    return null;
  }
  if (becameStrict && !simple) {
    return failTok(p, PE_USE_STRICT_NON_SIMPLE, st);
  }
  if (becameStrict) {
    if (!checkStrictNames(p, names)) {
      return null;
    }
    if (id !== null && !checkStrictNames(p, [id])) {
      return null;
    }
  }
  if (p.ctx.strict || !simple || flavor !== FL_NORMAL) {
    if (!checkUnique(p, names)) {
      return null;
    }
  }
  return makeFunction(
    id,
    params,
    body,
    isAsync,
    isGenerator,
    p.ctx.strict,
    simple,
    t.start[st],
    endOf(p, st),
    t.line[st],
    t.column[st],
  );
}

// After `(`, through `)`.
function parseFormalParams(p) {
  const t = p.t;
  const params = [];
  while (p.punct !== P_RPAREN) {
    if (p.punct === P_ELLIPSIS) {
      const rst = p.cur;
      if (!bump(p)) {
        return null;
      }
      const target = parseBindingTarget(p);
      if (target === null) {
        return null;
      }
      if (p.punct === P_EQ) {
        return failTok(p, PE_REST_PARAM_DEFAULT, p.cur);
      }
      params.push(makePatRest(target, t.start[rst], endOf(p, rst), t.line[rst], t.column[rst]));
      if (p.punct !== P_RPAREN) {
        return failTok(p, PE_REST_PARAM_LAST, p.cur);
      }
      break;
    }
    const e = parseBindingElement(p);
    if (e === null) {
      return null;
    }
    params.push(e);
    const c = eat(p, P_COMMA);
    if (c < 0) {
      return null;
    }
    if (c === 0) {
      break;
    }
  }
  if (!expect(p, P_RPAREN)) {
    return null;
  }
  return params;
}

// From `=>` on. `st` is the token the arrow starts at; `paramsStart` is
// the offset where its parameters start.
function parseArrowRest(p, st, params, isAsync, paramsStart) {
  const t = p.t;
  if (p.nl) {
    return failTok(p, PE_ARROW_NEWLINE, p.cur);
  }
  if (!expect(p, P_ARROW)) {
    return null;
  }
  if (p.lastYieldAwait >= 0 && p.lastYieldAwait >= paramsStart) {
    return failTok(p, PE_YIELD_AWAIT_ARROW_PARAMS, st);
  }
  const names = boundNamesOfList(params);
  if (isAsync) {
    if (p.lastAwaitIdent >= 0 && p.lastAwaitIdent >= paramsStart) {
      return failTok(p, PE_AWAIT_ASYNC_ARROW_PARAMS, st);
    }
    let i = 0;
    while (i < names.length) {
      if (unitsWord(names[i].name) === W_AWAIT) {
        return failNode(p, PE_AWAIT_PARAM_NAME, names[i]);
      }
      i = i + 1;
    }
  }
  if (!checkUnique(p, names)) {
    return null;
  }
  const simple = allIdents(params);
  const outer = p.ctx;
  const savedYieldAwait = p.lastYieldAwait;
  const savedAwaitIdent = p.lastAwaitIdent;
  const c = copyCtx(outer);
  c.inFunction = true;
  c.isAsync = isAsync;
  c.isGenerator = false;
  c.labels = [];
  c.loopDepth = 0;
  c.switchDepth = 0;
  p.ctx = c;
  pushScope(p, true);
  setParams(p.ee, names);
  const body = arrowBody(p, st, simple, names);
  const strict = p.ctx.strict;
  popScope(p);
  p.ctx = outer;
  p.lastYieldAwait = savedYieldAwait;
  p.lastAwaitIdent = savedAwaitIdent;
  if (body === null) {
    return null;
  }
  const end = endOf(p, st);
  const a = makeArrow(params, body, isAsync, strict, t.start[st], end, t.line[st], t.column[st]);
  return makeExprArrow(a, t.start[st], end, t.line[st], t.column[st]);
}

function arrowBody(p, st, simple, names) {
  if (p.punct === P_LBRACE) {
    const bst = p.cur;
    const savedIn = p.noIn;
    p.noIn = false;
    let body = null;
    if (bump(p)) {
      body = parseBody(p, true);
    }
    p.noIn = savedIn;
    if (body === null) {
      return null;
    }
    const becameStrict = p.becameStrict;
    if (!expect(p, P_RBRACE)) {
      return null;
    }
    if (becameStrict && !simple) {
      return failTok(p, PE_USE_STRICT_NON_SIMPLE, st);
    }
    if (becameStrict && !checkStrictNames(p, names)) {
      return null;
    }
    const t = p.t;
    return makeArrowBodyBlock(body, t.start[bst], endOf(p, bst), t.line[bst], t.column[bst]);
  }
  const e = parseAssign(p);
  if (e === null) {
    return null;
  }
  return makeArrowBodyExpr(e, e.start, e.end, e.line, e.col);
}

// ---- classes ----

function parseClass(p, isDecl, nameOptional) {
  const st = p.cur;
  if (!expectWord(p, W_CLASS)) {
    return null;
  }
  const outerStrict = p.ctx.strict;
  p.ctx.strict = true;
  const r = classInner(p, st, isDecl, nameOptional);
  p.ctx.strict = outerStrict;
  return r;
}

function classInner(p, st, isDecl, nameOptional) {
  const t = p.t;
  let id = null;
  if (p.kind === TK_IDENT && p.word !== W_EXTENDS) {
    const i = p.cur;
    const name = curName(p);
    const place = placeOf(p, i);
    if (!checkBindingIdent(p, nameWordOf(p, i), name, place)) {
      return null;
    }
    if (!bump(p)) {
      return null;
    }
    id = makeIdent(name, place.start, place.end, place.line, place.col);
  }
  if (id === null && isDecl && !nameOptional) {
    return failCur(p, PE_CLASS_NEEDS_NAME);
  }
  let superClass = null;
  if (p.word === W_EXTENDS) {
    if (!bump(p)) {
      return null;
    }
    const savedIn = p.noIn;
    p.noIn = false;
    superClass = parseLhs(p);
    p.noIn = savedIn;
    if (superClass === null) {
      return null;
    }
    if (isArrow(superClass)) {
      return failNode(p, PE_UNEXPECTED_ARROW, superClass);
    }
  }
  const derived = superClass !== null;
  if (!expect(p, P_LBRACE)) {
    return null;
  }
  const savedIn = p.noIn;
  p.noIn = false;
  openClass(p.ee);
  const body = classBody(p, derived);
  p.noIn = savedIn;
  if (body === null) {
    return null;
  }
  const missing = closeClass(p.ee);
  if (missing !== null) {
    return failNodeName(p, PE_PRIVATE_NOT_DEFINED, missing, missing.name);
  }
  return makeClass(id, superClass, body, t.start[st], endOf(p, st), t.line[st], t.column[st]);
}

function classBody(p, derived) {
  const body = [];
  let hasCtor = false;
  while (p.punct !== P_RBRACE) {
    const e = eat(p, P_SEMICOLON);
    if (e < 0) {
      return null;
    }
    if (e === 1) {
      continue;
    }
    if (p.kind === TK_EOF) {
      return unexpected(p);
    }
    const m = parseClassMember(p, derived);
    if (m === null) {
      return null;
    }
    if (m.kind === CM_METHOD && m.methodKind === MK_CONSTRUCTOR) {
      if (hasCtor) {
        return failNode(p, PE_ONE_CONSTRUCTOR, m);
      }
      hasCtor = true;
    }
    body.push(m);
  }
  if (!bump(p)) {
    return null;
  }
  return body;
}

// The rule private names share with the scope pass that can be checked
// here: `#constructor` is never a valid name.
function checkPrivateName(p, ident) {
  if (unitsWord(ident.name) === W_CONSTRUCTOR) {
    failNode(p, PE_PRIVATE_CONSTRUCTOR, ident);
    return false;
  }
  return true;
}

// Declare private member `#name` of the class body being read.
function privateMember(p, ident, kind, isStatic) {
  if (!checkPrivateName(p, ident)) {
    return false;
  }
  if (!declarePrivate(p.ee, ident.name, kind, isStatic)) {
    failNodeName(p, PE_DUPLICATE_PRIVATE, ident, ident.name);
    return false;
  }
  return true;
}

function parseClassMember(p, derived) {
  const t = p.t;
  const st = p.cur;
  let isStatic = false;
  if (p.word === W_STATIC) {
    const q = peek(p);
    if (q < 0) {
      return null;
    }
    if (tokPunct(p, q) === P_LBRACE) {
      if (!bump(p)) {
        return null;
      }
      const body = parseStaticBlock(p);
      if (body === null) {
        return null;
      }
      return makeMemberStaticBlock(body, t.start[st], endOf(p, st), t.line[st], t.column[st]);
    }
    if (startsKey(p, q, true) || tokPunct(p, q) === P_STAR) {
      if (!bump(p)) {
        return null;
      }
      isStatic = true;
    }
  }
  let isAsync = false;
  if (p.word === W_ASYNC) {
    const q = peek(p);
    if (q < 0) {
      return null;
    }
    if ((startsKey(p, q, true) || tokPunct(p, q) === P_STAR) && !tokNl(p, q)) {
      if (!bump(p)) {
        return null;
      }
      isAsync = true;
    }
  }
  const g = eat(p, P_STAR);
  if (g < 0) {
    return null;
  }
  const isGenerator = g === 1;
  let accessor = -1;
  if (!isAsync && !isGenerator && (p.word === W_GET || p.word === W_SET)) {
    const q = peek(p);
    if (q < 0) {
      return null;
    }
    if (startsKey(p, q, true)) {
      if (p.word === W_GET) {
        accessor = MK_GET;
      } else {
        accessor = MK_SET;
      }
      if (!bump(p)) {
        return null;
      }
    }
  }
  const key = parsePropKey(p, true);
  if (key === null) {
    return null;
  }
  const kw = keyWord(key);
  if (p.punct === P_LPAREN) {
    const isCtor = !isStatic && kw === W_CONSTRUCTOR;
    if (isCtor && (accessor >= 0 || isAsync || isGenerator)) {
      return failTok(p, PE_CONSTRUCTOR_SPECIAL, st);
    }
    if (isStatic && kw === W_PROTOTYPE) {
      return failTok(p, PE_STATIC_PROTOTYPE_METHOD, st);
    }
    let kind = MK_METHOD;
    let flavor = FL_METHOD;
    if (isCtor) {
      kind = MK_CONSTRUCTOR;
      flavor = FL_CONSTRUCTOR;
      if (derived) {
        flavor = FL_DERIVED_CONSTRUCTOR;
      }
    } else if (accessor === MK_GET) {
      kind = MK_GET;
      flavor = FL_GETTER;
    } else if (accessor === MK_SET) {
      kind = MK_SET;
      flavor = FL_SETTER;
    }
    if (key.kind === PK_PRIVATE) {
      let pk = PRIV_METHOD;
      if (accessor === MK_GET) {
        pk = PRIV_GET;
      } else if (accessor === MK_SET) {
        pk = PRIV_SET;
      }
      if (!privateMember(p, key.ident, pk, isStatic)) {
        return null;
      }
    }
    const value = parseFunctionRest(p, p.cur, null, isAsync, isGenerator, flavor);
    if (value === null) {
      return null;
    }
    return makeMemberMethod(key, kind, isStatic, value, t.start[st], endOf(p, st), t.line[st], t.column[st]);
  }
  if (isAsync || isGenerator || accessor >= 0) {
    return unexpected(p);
  }
  if (kw === W_CONSTRUCTOR) {
    return failTok(p, PE_FIELD_CONSTRUCTOR, st);
  }
  if (isStatic && kw === W_PROTOTYPE) {
    return failTok(p, PE_STATIC_PROTOTYPE_FIELD, st);
  }
  if (key.kind === PK_PRIVATE && !privateMember(p, key.ident, PRIV_FIELD, isStatic)) {
    return null;
  }
  let value = null;
  const e = eat(p, P_EQ);
  if (e < 0) {
    return null;
  }
  if (e === 1) {
    const outer = p.ctx;
    const savedYieldAwait = p.lastYieldAwait;
    const savedAwaitIdent = p.lastAwaitIdent;
    const c = newCtx(true);
    c.inFunction = true;
    c.superProp = true;
    c.newTarget = true;
    c.classInit = true;
    p.ctx = c;
    pushScope(p, true);
    value = parseAssign(p);
    popScope(p);
    p.ctx = outer;
    p.lastYieldAwait = savedYieldAwait;
    p.lastAwaitIdent = savedAwaitIdent;
    if (value === null) {
      return null;
    }
  }
  if (!semicolon(p)) {
    return null;
  }
  return makeMemberField(key, isStatic, value, t.start[st], endOf(p, st), t.line[st], t.column[st]);
}

function parseStaticBlock(p) {
  const outer = p.ctx;
  const savedYieldAwait = p.lastYieldAwait;
  const savedAwaitIdent = p.lastAwaitIdent;
  const c = newCtx(true);
  c.superProp = true;
  c.newTarget = true;
  c.classInit = true;
  c.staticBlock = true;
  p.ctx = c;
  pushScope(p, true);
  const r = staticBlockBody(p);
  popScope(p);
  p.ctx = outer;
  p.lastYieldAwait = savedYieldAwait;
  p.lastAwaitIdent = savedAwaitIdent;
  return r;
}

function staticBlockBody(p) {
  if (!expect(p, P_LBRACE)) {
    return null;
  }
  const body = [];
  while (p.punct !== P_RBRACE) {
    if (p.kind === TK_EOF) {
      return unexpected(p);
    }
    const s = parseStatementListItem(p);
    if (s === null) {
      return null;
    }
    body.push(s);
  }
  if (!bump(p)) {
    return null;
  }
  return body;
}

function parsePropKey(p, allowPrivate) {
  const t = p.t;
  const i = p.cur;
  const k = p.kind;
  if (k === TK_IDENT) {
    const id = identOf(p, i);
    if (!bump(p)) {
      return null;
    }
    return makeKeyIdent(id, id.start, id.end, id.line, id.col);
  }
  if (k === TK_STRING) {
    if (!checkLegacy(p, i)) {
      return null;
    }
    const v = stringValue(t, p.src, i);
    if (!bump(p)) {
      return null;
    }
    return makeKeyStr(v, t.start[i], t.end[i], t.line[i], t.column[i]);
  }
  if (k === TK_NUMBER) {
    if (!checkLegacy(p, i)) {
      return null;
    }
    const v = numberValue(t, i);
    if (!bump(p)) {
      return null;
    }
    return makeKeyNum(v, t.start[i], t.end[i], t.line[i], t.column[i]);
  }
  if (k === TK_BIGINT) {
    const radix = bigintRadix(t, i);
    const digits = bigintDigits(t, i);
    if (!bump(p)) {
      return null;
    }
    return makeKeyBigint(radix, digits, t.start[i], t.end[i], t.line[i], t.column[i]);
  }
  if (k === TK_PRIVATE && allowPrivate) {
    const id = identOf(p, i);
    if (!bump(p)) {
      return null;
    }
    return makeKeyPrivate(id, id.start, id.end, id.line, id.col);
  }
  if (p.punct === P_LBRACKET) {
    if (!bump(p)) {
      return null;
    }
    const savedIn = p.noIn;
    p.noIn = false;
    const e = parseAssign(p);
    p.noIn = savedIn;
    if (e === null || !expect(p, P_RBRACKET)) {
      return null;
    }
    return makeKeyComputed(e, t.start[i], endOf(p, i), t.line[i], t.column[i]);
  }
  return unexpected(p);
}

// ---- binding patterns ----

function parseBindingTarget(p) {
  if (!enter(p)) {
    return null;
  }
  const r = bindingTargetInner(p);
  leave(p);
  return r;
}

function bindingTargetInner(p) {
  const t = p.t;
  const st = p.cur;
  if (p.kind === TK_IDENT) {
    const name = curName(p);
    const place = placeOf(p, st);
    if (!checkBindingIdent(p, nameWordOf(p, st), name, place)) {
      return null;
    }
    if (!bump(p)) {
      return null;
    }
    return patIdentAt(name, place);
  }
  if (p.punct === P_LBRACKET) {
    if (!bump(p)) {
      return null;
    }
    const savedIn = p.noIn;
    p.noIn = false;
    const items = arrayBindingItems(p);
    p.noIn = savedIn;
    if (items === null) {
      return null;
    }
    return makePatArray(items, t.start[st], endOf(p, st), t.line[st], t.column[st]);
  }
  if (p.punct === P_LBRACE) {
    if (!bump(p)) {
      return null;
    }
    const savedIn = p.noIn;
    p.noIn = false;
    const props = objectBindingProps(p);
    p.noIn = savedIn;
    if (props === null) {
      return null;
    }
    return makePatObject(props, t.start[st], endOf(p, st), t.line[st], t.column[st]);
  }
  return unexpected(p);
}

function arrayBindingItems(p) {
  const t = p.t;
  const items = [];
  while (p.punct !== P_RBRACKET) {
    const c = eat(p, P_COMMA);
    if (c < 0) {
      return null;
    }
    if (c === 1) {
      items.push(null);
      continue;
    }
    if (p.punct === P_ELLIPSIS) {
      const rst = p.cur;
      if (!bump(p)) {
        return null;
      }
      const target = parseBindingTarget(p);
      if (target === null) {
        return null;
      }
      items.push(makePatRest(target, t.start[rst], endOf(p, rst), t.line[rst], t.column[rst]));
      if (p.punct !== P_RBRACKET) {
        return failTok(p, PE_REST_ELEMENT_LAST, p.cur);
      }
      break;
    }
    const e = parseBindingElement(p);
    if (e === null) {
      return null;
    }
    items.push(e);
    if (p.punct !== P_RBRACKET && !expect(p, P_COMMA)) {
      return null;
    }
  }
  if (!bump(p)) {
    return null;
  }
  return items;
}

function objectBindingProps(p) {
  const props = [];
  while (p.punct !== P_RBRACE) {
    if (p.punct === P_ELLIPSIS) {
      if (!bump(p)) {
        return null;
      }
      const i = p.cur;
      if (p.kind !== TK_IDENT) {
        return failTok(p, PE_OBJECT_REST_PLAIN_NAME, i);
      }
      const name = curName(p);
      const place = placeOf(p, i);
      if (!checkBindingIdent(p, nameWordOf(p, i), name, place)) {
        return null;
      }
      if (!bump(p)) {
        return null;
      }
      const arg = patIdentAt(name, place);
      props.push(makePatPropRest(arg, arg.start, arg.end, arg.line, arg.col));
      if (p.punct !== P_RBRACE) {
        return failTok(p, PE_REST_ELEMENT_LAST, p.cur);
      }
      break;
    }
    const kst = p.cur;
    const key = parsePropKey(p, false);
    if (key === null) {
      return null;
    }
    const c = eat(p, P_COLON);
    if (c < 0) {
      return null;
    }
    if (c === 1) {
      const value = parseBindingElement(p);
      if (value === null) {
        return null;
      }
      props.push(makePatPropProp(key, value, false, key.start, value.end, key.line, key.col));
    } else {
      if (key.kind !== PK_IDENT) {
        return failTok(p, PE_EXPECTED_COLON, p.cur);
      }
      const id = key.ident;
      if (!checkBindingIdent(p, unitsWord(id.name), id.name, id)) {
        return null;
      }
      let value = patIdentAt(id.name, placeOf(p, kst));
      const e = eat(p, P_EQ);
      if (e < 0) {
        return null;
      }
      if (e === 1) {
        const d = parseAssign(p);
        if (d === null) {
          return null;
        }
        value = makePatAssign(value, d, value.start, maxEnd(value, d), value.line, value.col);
      }
      props.push(makePatPropProp(key, value, true, key.start, value.end, key.line, key.col));
    }
    if (p.punct !== P_RBRACE && !expect(p, P_COMMA)) {
      return null;
    }
  }
  if (!bump(p)) {
    return null;
  }
  return props;
}

function parseBindingElement(p) {
  const t = p.t;
  const st = p.cur;
  const target = parseBindingTarget(p);
  if (target === null) {
    return null;
  }
  const e = eat(p, P_EQ);
  if (e < 0) {
    return null;
  }
  if (e === 1) {
    const savedIn = p.noIn;
    p.noIn = false;
    const d = parseAssign(p);
    p.noIn = savedIn;
    if (d === null) {
      return null;
    }
    return makePatAssign(target, d, t.start[st], endOf(p, st), t.line[st], t.column[st]);
  }
  return target;
}

// ---- expressions ----

function parseExpressionIn(p) {
  const saved = p.noIn;
  p.noIn = false;
  const r = parseExpression(p);
  p.noIn = saved;
  return r;
}

function parseAssignIn(p) {
  const saved = p.noIn;
  p.noIn = false;
  const r = parseAssignInner(p, false);
  p.noIn = saved;
  return r;
}

// Expression, with commas.
function parseExpression(p) {
  const first = parseAssignInner(p, false);
  if (first === null) {
    return null;
  }
  if (p.punct !== P_COMMA) {
    return first;
  }
  const items = [first];
  while (p.punct === P_COMMA) {
    if (!bump(p)) {
      return null;
    }
    const x = parseAssignInner(p, false);
    if (x === null) {
      return null;
    }
    items.push(x);
  }
  const last = items[items.length - 1];
  return makeExprSeq(items, first.start, maxEnd(first, last), first.line, first.col);
}

function parseAssign(p) {
  return parseAssignInner(p, false);
}

// AssignmentExpression (with the conditional operator). `cover`: an
// object or array literal may keep a deferred cover error, because the
// caller may still turn it into a pattern.
function parseAssignInner(p, cover) {
  if (!enter(p)) {
    return null;
  }
  const r = assignBody(p, cover);
  leave(p);
  return r;
}

function assignBody(p, cover) {
  if (p.ctx.isGenerator && p.word === W_YIELD) {
    return parseYield(p);
  }
  const savedCover = p.coverErr;
  p.coverErr = null;
  let lhs = parseBinary(p, 0);
  if (lhs === null) {
    return null;
  }
  if (isArrow(lhs)) {
    p.coverErr = savedCover;
    return lhs;
  }
  if (p.punct === P_QUESTION) {
    if (!bump(p)) {
      return null;
    }
    const cons = parseAssignIn(p);
    if (cons === null || !expect(p, P_COLON)) {
      return null;
    }
    const alt = parseAssignInner(p, false);
    if (alt === null) {
      return null;
    }
    lhs = makeExprCond(lhs, cons, alt, lhs.start, maxEnd(lhs, alt), lhs.line, lhs.col);
  }
  const op = assignOp(p.punct);
  if (op >= 0) {
    return finishAssign(p, lhs, op, savedCover);
  }
  if (p.coverErr !== null) {
    const e = p.coverErr;
    if (cover && (lhs.kind === E_OBJECT || lhs.kind === E_ARRAY)) {
      if (savedCover !== null) {
        p.coverErr = savedCover;
      } else {
        p.coverErr = e;
      }
      return lhs;
    }
    return raiseCover(p, e);
  }
  p.coverErr = savedCover;
  return lhs;
}

function assignOp(punct) {
  if (punct === P_EQ) {
    return A_ASSIGN;
  }
  if (punct < P_PLUS_EQ || punct > P_QUESTION_QUESTION_EQ) {
    if (punct === P_SLASH_EQ) {
      return A_DIV;
    }
    return -1;
  }
  if (punct === P_PLUS_EQ) {
    return A_ADD;
  }
  if (punct === P_MINUS_EQ) {
    return A_SUB;
  }
  if (punct === P_STAR_EQ) {
    return A_MUL;
  }
  if (punct === P_PERCENT_EQ) {
    return A_MOD;
  }
  if (punct === P_STAR_STAR_EQ) {
    return A_EXP;
  }
  if (punct === P_SHL_EQ) {
    return A_SHL;
  }
  if (punct === P_SHR_EQ) {
    return A_SHR;
  }
  if (punct === P_USHR_EQ) {
    return A_USHR;
  }
  if (punct === P_AMP_EQ) {
    return A_BIT_AND;
  }
  if (punct === P_PIPE_EQ) {
    return A_BIT_OR;
  }
  if (punct === P_CARET_EQ) {
    return A_BIT_XOR;
  }
  if (punct === P_AMP_AMP_EQ) {
    return A_AND;
  }
  if (punct === P_PIPE_PIPE_EQ) {
    return A_OR;
  }
  return A_NULLISH;
}

function finishAssign(p, lhs, op, savedCover) {
  let target = null;
  if (op === A_ASSIGN && (lhs.kind === E_OBJECT || lhs.kind === E_ARRAY)) {
    target = exprToPattern(p, lhs, false);
    if (target === null) {
      return null;
    }
    p.coverErr = null;
  } else {
    if (p.coverErr !== null) {
      return raiseCover(p, p.coverErr);
    }
    target = simpleTarget(p, lhs);
    if (target === null) {
      return null;
    }
  }
  if (!bump(p)) {
    return null;
  }
  const value = parseAssignInner(p, false);
  if (value === null) {
    return null;
  }
  p.coverErr = savedCover;
  return makeExprAssign(op, target, value, target.start, maxEnd(target, value), target.line, target.col);
}

function parseYield(p) {
  const t = p.t;
  const st = p.cur;
  if (!bump(p)) {
    return null;
  }
  p.lastYieldAwait = t.start[st];
  const q = p.punct;
  const noArg =
    p.nl ||
    p.kind === TK_EOF ||
    q === P_RPAREN ||
    q === P_RBRACKET ||
    q === P_RBRACE ||
    q === P_COMMA ||
    q === P_SEMICOLON ||
    q === P_COLON ||
    q === P_QUESTION ||
    p.word === W_IN ||
    p.word === W_OF;
  if (noArg) {
    return makeExprYield(null, false, t.start[st], t.end[st], t.line[st], t.column[st]);
  }
  const d = eat(p, P_STAR);
  if (d < 0) {
    return null;
  }
  const arg = parseAssignInner(p, false);
  if (arg === null) {
    return null;
  }
  return makeExprYield(arg, d === 1, t.start[st], endOf(p, st), t.line[st], t.column[st]);
}

// Precedence of the binary operator at the current token, 0 for none.
function binaryPrec(p) {
  const q = p.punct;
  if (q >= 0) {
    if (q === P_QUESTION_QUESTION) {
      return 1;
    }
    if (q === P_PIPE_PIPE) {
      return 2;
    }
    if (q === P_AMP_AMP) {
      return 3;
    }
    if (q === P_PIPE) {
      return 4;
    }
    if (q === P_CARET) {
      return 5;
    }
    if (q === P_AMP) {
      return 6;
    }
    if (q === P_EQ_EQ || q === P_NOT_EQ || q === P_EQ_EQ_EQ || q === P_NOT_EQ_EQ) {
      return 7;
    }
    if (q === P_LT || q === P_GT || q === P_LT_EQ || q === P_GT_EQ) {
      return 8;
    }
    if (q === P_SHL || q === P_SHR || q === P_USHR) {
      return 9;
    }
    if (q === P_PLUS || q === P_MINUS) {
      return 10;
    }
    if (q === P_STAR || q === P_SLASH || q === P_PERCENT) {
      return 11;
    }
    if (q === P_STAR_STAR) {
      return 12;
    }
    return 0;
  }
  if (p.word === W_INSTANCEOF) {
    return 8;
  }
  if (p.word === W_IN && !p.noIn) {
    return 8;
  }
  return 0;
}

// Logical operators come back as LOGICAL + their code.
const LOGICAL = 100;

function binaryOp(p) {
  const q = p.punct;
  if (q === P_QUESTION_QUESTION) {
    return LOGICAL + L_NULLISH;
  }
  if (q === P_PIPE_PIPE) {
    return LOGICAL + L_OR;
  }
  if (q === P_AMP_AMP) {
    return LOGICAL + L_AND;
  }
  if (q === P_PIPE) {
    return B_BIT_OR;
  }
  if (q === P_CARET) {
    return B_BIT_XOR;
  }
  if (q === P_AMP) {
    return B_BIT_AND;
  }
  if (q === P_EQ_EQ) {
    return B_EQ;
  }
  if (q === P_NOT_EQ) {
    return B_NOT_EQ;
  }
  if (q === P_EQ_EQ_EQ) {
    return B_STRICT_EQ;
  }
  if (q === P_NOT_EQ_EQ) {
    return B_STRICT_NOT_EQ;
  }
  if (q === P_LT) {
    return B_LT;
  }
  if (q === P_GT) {
    return B_GT;
  }
  if (q === P_LT_EQ) {
    return B_LT_EQ;
  }
  if (q === P_GT_EQ) {
    return B_GT_EQ;
  }
  if (q === P_SHL) {
    return B_SHL;
  }
  if (q === P_SHR) {
    return B_SHR;
  }
  if (q === P_USHR) {
    return B_USHR;
  }
  if (q === P_PLUS) {
    return B_ADD;
  }
  if (q === P_MINUS) {
    return B_SUB;
  }
  if (q === P_STAR) {
    return B_MUL;
  }
  if (q === P_SLASH) {
    return B_DIV;
  }
  if (q === P_PERCENT) {
    return B_MOD;
  }
  if (q === P_STAR_STAR) {
    return B_EXP;
  }
  if (p.word === W_INSTANCEOF) {
    return B_INSTANCEOF;
  }
  return B_IN;
}

// Whether combining `e` with logical operator `op` mixes `??` with `&&`
// or `||`.
function mixesNullish(e, op) {
  if (e.kind !== E_LOGICAL) {
    return false;
  }
  if (op === L_NULLISH) {
    return e.op !== L_NULLISH;
  }
  return e.op === L_NULLISH;
}

// Binary operators of precedence minPrec and up, by precedence climbing.
// Each operator of a chain counts one nesting level until the chain ends.
function parseBinary(p, minPrec) {
  const t = p.t;
  let left = null;
  if (p.kind === TK_PRIVATE) {
    const i = p.cur;
    if (minPrec > 8) {
      return unexpected(p);
    }
    const name = identOf(p, i);
    if (!bump(p)) {
      return null;
    }
    if (p.word !== W_IN || p.noIn) {
      return failTok(p, PE_PRIVATE_IN, i);
    }
    if (!referencePrivate(p.ee, name)) {
      return failNodeName(p, PE_PRIVATE_NOT_DEFINED, name, name.name);
    }
    if (!bump(p) || !enter(p)) {
      return null;
    }
    const right = parseBinary(p, 9);
    leave(p);
    if (right === null) {
      return null;
    }
    left = makeExprPrivateIn(name, right, name.start, maxEnd(name, right), name.line, name.col);
  } else {
    left = parseUnary(p);
    if (left === null) {
      return null;
    }
  }
  let links = 0;
  while (!isArrow(left)) {
    const prec = binaryPrec(p);
    if (prec === 0 || prec < minPrec) {
      break;
    }
    const op = binaryOp(p);
    if (op === B_EXP && (left.kind === E_UNARY || left.kind === E_AWAIT)) {
      return failNode(p, PE_EXP_UNARY, left);
    }
    if (!bump(p) || !enter(p)) {
      return null;
    }
    links = links + 1;
    let right = null;
    if (op === B_EXP) {
      right = parseBinary(p, prec);
    } else {
      right = parseBinary(p, prec + 1);
    }
    if (right === null) {
      return null;
    }
    if (isArrow(right)) {
      return failNode(p, PE_ARROW_OPERAND, right);
    }
    const end = maxEnd(left, right);
    if (op >= LOGICAL) {
      const lop = op - LOGICAL;
      if (mixesNullish(left, lop) || mixesNullish(right, lop)) {
        return failNode(p, PE_NULLISH_MIX, left);
      }
      left = makeExprLogical(lop, left, right, left.start, end, left.line, left.col);
    } else {
      left = makeExprBinary(op, left, right, left.start, end, left.line, left.col);
    }
  }
  p.depth = p.depth - links;
  if (p.depth < 0) {
    p.depth = 0;
  }
  return left;
}

// Prefix operator kinds on the operator stack.
const PRE_UNARY = 0;
const PRE_UPDATE = 1;
const PRE_AWAIT = 2;

function unaryOpOf(p) {
  const q = p.punct;
  if (q === P_BANG) {
    return U_NOT;
  }
  if (q === P_TILDE) {
    return U_BIT_NOT;
  }
  if (q === P_PLUS) {
    return U_PLUS;
  }
  if (q === P_MINUS) {
    return U_MINUS;
  }
  if (p.word === W_TYPEOF) {
    return U_TYPEOF;
  }
  if (p.word === W_VOID) {
    return U_VOID;
  }
  if (p.word === W_DELETE) {
    return U_DELETE;
  }
  return -1;
}

// UnaryExpression and UpdateExpression. A run of prefix operators is
// read in a loop onto p.prefix, then applied from the inside out, with
// the same checks (and nesting count) as one call per operator.
function parseUnary(p) {
  const t = p.t;
  const stack = p.prefix;
  const base = stack.length;
  while (true) {
    const st = p.cur;
    const u = unaryOpOf(p);
    if (u >= 0) {
      stack.push(PRE_UNARY);
      stack.push(u);
    } else if (p.punct === P_PLUS_PLUS || p.punct === P_MINUS_MINUS) {
      stack.push(PRE_UPDATE);
      if (p.punct === P_PLUS_PLUS) {
        stack.push(UP_INC);
      } else {
        stack.push(UP_DEC);
      }
    } else if (p.word === W_AWAIT && awaitIsKeyword(p)) {
      stack.push(PRE_AWAIT);
      stack.push(0);
    } else {
      break;
    }
    stack.push(st);
    if (!bump(p)) {
      return null;
    }
    if (stack[stack.length - 3] === PRE_AWAIT) {
      p.lastYieldAwait = t.start[st];
    }
    if (!enter(p)) {
      return null;
    }
  }
  let e = parsePostfix(p);
  if (e === null) {
    return null;
  }
  while (stack.length > base) {
    const st = stack.pop();
    const op = stack.pop();
    const kind = stack.pop();
    leave(p);
    if (kind === PRE_UPDATE) {
      if (!checkSimpleTarget(p, e)) {
        return null;
      }
      e = makeExprUpdate(op, true, e, t.start[st], endOf(p, st), t.line[st], t.column[st]);
    } else {
      if (isArrow(e)) {
        return failNode(p, PE_ARROW_OPERAND, e);
      }
      if (kind === PRE_AWAIT) {
        e = makeExprAwait(e, t.start[st], endOf(p, st), t.line[st], t.column[st]);
      } else {
        if (op === U_DELETE && !checkDelete(p, e)) {
          return null;
        }
        e = makeExprUnary(op, e, t.start[st], endOf(p, st), t.line[st], t.column[st]);
      }
    }
  }
  return e;
}

function checkDelete(p, arg) {
  const inner = unparen(arg);
  if (p.ctx.strict && inner.kind === E_IDENT) {
    failNode(p, PE_DELETE_NAME, arg);
    return false;
  }
  let probe = inner;
  if (probe.kind === E_OPT_CHAIN) {
    probe = probe.expr;
  }
  if (probe.kind === E_MEMBER && probe.prop.kind === MP_PRIVATE) {
    failNode(p, PE_DELETE_PRIVATE, arg);
    return false;
  }
  return true;
}

function parsePostfix(p) {
  const t = p.t;
  const st = p.cur;
  const e = parseLhs(p);
  if (e === null) {
    return null;
  }
  if ((p.punct === P_PLUS_PLUS || p.punct === P_MINUS_MINUS) && !p.nl && !isArrow(e)) {
    let op = UP_DEC;
    if (p.punct === P_PLUS_PLUS) {
      op = UP_INC;
    }
    if (!checkSimpleTarget(p, e) || !bump(p)) {
      return null;
    }
    return makeExprUpdate(op, false, e, t.start[st], endOf(p, st), t.line[st], t.column[st]);
  }
  return e;
}

function checkSimpleTarget(p, e) {
  const inner = unparen(e);
  if (inner.kind === E_IDENT) {
    const w = unitsWord(inner.name);
    if (p.ctx.strict && (w === W_EVAL || w === W_ARGUMENTS)) {
      failNodeName(p, PE_ASSIGN_STRICT, e, inner.name);
      return false;
    }
    return true;
  }
  if (inner.kind === E_MEMBER) {
    return true;
  }
  failNode(p, PE_INVALID_TARGET, e);
  return false;
}

function simpleTarget(p, e) {
  if (!checkSimpleTarget(p, e)) {
    return null;
  }
  if (e.kind === E_IDENT) {
    return makePatIdent(e.name, e.start, e.end, e.line, e.col);
  }
  return makePatExpr(e, e.start, e.end, e.line, e.col);
}

// Target of a for-in or for-of head.
function exprToTarget(p, e) {
  if (e.kind === E_OBJECT || e.kind === E_ARRAY) {
    return exprToPattern(p, e, false);
  }
  return simpleTarget(p, e);
}

// A cover expression as a pattern. `binding`: for arrow parameters, where
// only names may be bound.
function exprToPattern(p, e, binding) {
  if (!enter(p)) {
    return null;
  }
  const r = exprToPatternInner(p, e, binding);
  leave(p);
  return r;
}

function exprToPatternInner(p, e, binding) {
  const k = e.kind;
  if (!binding && k === E_MEMBER) {
    return makePatExpr(e, e.start, e.end, e.line, e.col);
  }
  if (!binding && k === E_PAREN) {
    const inner = unparen(e);
    if (inner.kind === E_IDENT || inner.kind === E_MEMBER) {
      return simpleTarget(p, e);
    }
    return failNode(p, PE_INVALID_DESTRUCTURING, e);
  }
  if (k === E_IDENT) {
    const w = unitsWord(e.name);
    if (binding) {
      if (!checkBindingIdent(p, w, e.name, e)) {
        return null;
      }
    } else if (p.ctx.strict && (w === W_EVAL || w === W_ARGUMENTS)) {
      return failNodeName(p, PE_ASSIGN_STRICT, e, e.name);
    }
    return makePatIdent(e.name, e.start, e.end, e.line, e.col);
  }
  if (k === E_ARRAY) {
    const items = e.items;
    const n = items.length;
    const out = [];
    let i = 0;
    while (i < n) {
      const x = items[i];
      if (x === null) {
        out.push(null);
      } else if (x.kind === E_SPREAD) {
        if (i + 1 !== n || isBadRest(p, x.start)) {
          return failNode(p, PE_REST_ELEMENT_LAST, x);
        }
        if (x.arg.kind === E_ASSIGN) {
          return failNode(p, PE_REST_ELEMENT_DEFAULT, x);
        }
        const target = exprToPattern(p, x.arg, binding);
        if (target === null) {
          return null;
        }
        out.push(makePatRest(target, x.start, x.end, x.line, x.col));
      } else {
        const y = exprToPattern(p, x, binding);
        if (y === null) {
          return null;
        }
        out.push(y);
      }
      i = i + 1;
    }
    return makePatArray(out, e.start, e.end, e.line, e.col);
  }
  if (k === E_OBJECT) {
    const props = e.props;
    const n = props.length;
    const out = [];
    let i = 0;
    while (i < n) {
      const pr = props[i];
      if (pr.kind === PR_INIT) {
        const key = pr.key;
        const v = pr.value;
        const shorthand =
          key.kind === PK_IDENT && v.kind === E_ASSIGN && v.target.start === key.ident.start && v.target.end === key.ident.end;
        const value = exprToPattern(p, v, binding);
        if (value === null) {
          return null;
        }
        out.push(makePatPropProp(key, value, shorthand, pr.start, pr.end, pr.line, pr.col));
      } else if (pr.kind === PR_SHORTHAND) {
        const id = pr.ident;
        const w = unitsWord(id.name);
        if (binding) {
          if (!checkBindingIdent(p, w, id.name, id)) {
            return null;
          }
        } else if (p.ctx.strict && (w === W_EVAL || w === W_ARGUMENTS)) {
          return failNodeName(p, PE_ASSIGN_STRICT, id, id.name);
        }
        const value = makePatIdent(id.name, id.start, id.end, id.line, id.col);
        const key = makeKeyIdent(id, id.start, id.end, id.line, id.col);
        out.push(makePatPropProp(key, value, true, pr.start, pr.end, pr.line, pr.col));
      } else if (pr.kind === PR_SPREAD) {
        if (i + 1 !== n || isBadRest(p, pr.start)) {
          return failNode(p, PE_REST_ELEMENT_LAST, pr);
        }
        const ak = pr.arg.kind;
        let ok = ak === E_IDENT;
        if (ak === E_MEMBER || ak === E_PAREN) {
          ok = !binding;
        }
        if (!ok) {
          return failNode(p, PE_OBJECT_REST_NAME, pr);
        }
        const target = exprToPattern(p, pr.arg, binding);
        if (target === null) {
          return null;
        }
        out.push(makePatPropRest(target, pr.start, pr.end, pr.line, pr.col));
      } else {
        return failNode(p, PE_METHOD_DESTRUCTURING, pr);
      }
      i = i + 1;
    }
    return makePatObject(out, e.start, e.end, e.line, e.col);
  }
  if (k === E_ASSIGN && e.op === A_ASSIGN) {
    if (binding && !validateBindingPat(p, e.target)) {
      return null;
    }
    return makePatAssign(e.target, e.value, e.start, e.end, e.line, e.col);
  }
  return failNode(p, PE_INVALID_DESTRUCTURING, e);
}

// A pattern made by an assignment, checked again as a binding.
function validateBindingPat(p, pat) {
  if (!enter(p)) {
    return false;
  }
  const r = validateBindingInner(p, pat);
  leave(p);
  return r;
}

function validateBindingInner(p, pat) {
  const k = pat.kind;
  if (k === PT_IDENT) {
    return checkBindingIdent(p, unitsWord(pat.name), pat.name, pat);
  }
  if (k === PT_EXPR) {
    failNode(p, PE_INVALID_BINDING, pat);
    return false;
  }
  if (k === PT_ARRAY) {
    let i = 0;
    while (i < pat.items.length) {
      if (pat.items[i] !== null && !validateBindingPat(p, pat.items[i])) {
        return false;
      }
      i = i + 1;
    }
    return true;
  }
  if (k === PT_OBJECT) {
    let i = 0;
    while (i < pat.props.length) {
      const pp = pat.props[i];
      let sub = pp.arg;
      if (pp.kind === PP_PROP) {
        sub = pp.value;
      }
      if (!validateBindingPat(p, sub)) {
        return false;
      }
      i = i + 1;
    }
    return true;
  }
  if (k === PT_ASSIGN) {
    return validateBindingPat(p, pat.target);
  }
  return validateBindingPat(p, pat.arg);
}

// ---- member, call, new ----

function parseLhs(p) {
  const st = p.cur;
  let e = null;
  if (p.word === W_NEW) {
    e = parseNew(p);
  } else if (p.word === W_SUPER) {
    e = parseSuper(p, true);
  } else if (p.word === W_IMPORT) {
    e = parseImportExpr(p);
  } else if (p.punct === P_LPAREN) {
    e = parseParen(p);
  } else if (p.punct === P_LBRACKET) {
    e = parseArray(p);
  } else if (p.punct === P_LBRACE) {
    e = parseObject(p);
  } else if (p.kind === TK_TEMPLATE) {
    const q = parseTemplate(p, false);
    if (q === null) {
      return null;
    }
    const t = p.t;
    e = makeExprTemplate(q, t.start[st], endOf(p, st), t.line[st], t.column[st]);
  } else {
    e = parsePrimary(p);
  }
  if (e === null) {
    return null;
  }
  if (isArrow(e)) {
    return e;
  }
  return parseCallTail(p, e, st, true);
}

function parseSuper(p, allowCall) {
  const t = p.t;
  const i = p.cur;
  if (!bump(p)) {
    return null;
  }
  if (p.punct === P_LPAREN && allowCall) {
    if (!p.ctx.superCall) {
      return failTok(p, PE_SUPER_CALL, i);
    }
  } else if (p.punct === P_DOT || p.punct === P_LBRACKET) {
    if (!p.ctx.superProp) {
      return failTok(p, PE_SUPER_PROP, i);
    }
  } else {
    return failTok(p, PE_SUPER_FOLLOW, i);
  }
  return makeExprSuper(t.start[i], t.end[i], t.line[i], t.column[i]);
}

function parseNew(p) {
  const t = p.t;
  const st = p.cur;
  if (!bump(p)) {
    return null;
  }
  if (p.punct === P_DOT) {
    if (!bump(p)) {
      return null;
    }
    if (p.word !== W_TARGET) {
      return failTok(p, PE_EXPECTED_NEW_TARGET, p.cur);
    }
    if (!bump(p)) {
      return null;
    }
    if (!p.ctx.newTarget) {
      return failTok(p, PE_NEW_TARGET_OUTSIDE, st);
    }
    return makeExprNewTarget(t.start[st], endOf(p, st), t.line[st], t.column[st]);
  }
  const cst = p.cur;
  let callee = null;
  if (p.word === W_NEW) {
    if (!enter(p)) {
      return null;
    }
    callee = parseNew(p);
    leave(p);
  } else if (p.word === W_SUPER) {
    callee = parseSuper(p, false);
  } else if (p.word === W_IMPORT) {
    const q = peek(p);
    if (q < 0) {
      return null;
    }
    if (tokPunct(p, q) !== P_DOT) {
      return failTok(p, PE_NEW_IMPORT, p.cur);
    }
    callee = parseImportExpr(p);
  } else if (p.punct === P_LPAREN) {
    callee = parseParen(p);
  } else if (p.punct === P_LBRACKET) {
    callee = parseArray(p);
  } else if (p.punct === P_LBRACE) {
    callee = parseObject(p);
  } else if (p.kind === TK_TEMPLATE) {
    const q = parseTemplate(p, false);
    if (q === null) {
      return null;
    }
    callee = makeExprTemplate(q, t.start[cst], endOf(p, cst), t.line[cst], t.column[cst]);
  } else {
    callee = parsePrimary(p);
  }
  if (callee === null) {
    return null;
  }
  if (isArrow(callee)) {
    return failNode(p, PE_ARROW_NEW, callee);
  }
  callee = parseCallTail(p, callee, cst, false);
  if (callee === null) {
    return null;
  }
  let args = [];
  if (p.punct === P_LPAREN) {
    args = parseArgs(p);
    if (args === null) {
      return null;
    }
  }
  return makeExprNew(callee, args, t.start[st], endOf(p, st), t.line[st], t.column[st]);
}

// Member accesses, calls and tagged templates after `e`, which started at
// token st. Each link counts one nesting level until the chain ends.
function parseCallTail(p, e0, st, allowCall) {
  const t = p.t;
  const s0 = t.start[st];
  const l0 = t.line[st];
  const c0 = t.column[st];
  let e = e0;
  let links = 0;
  let inChain = false;
  const isSuper = e0.kind === E_SUPER;
  while (true) {
    let next = null;
    if (p.punct === P_DOT) {
      if (!bump(p)) {
        return null;
      }
      const prop = parseMemberName(p);
      if (prop === null) {
        return null;
      }
      next = makeExprMember(e, prop, false, s0, s0, l0, c0);
    } else if (p.punct === P_QUESTION_DOT) {
      if (!allowCall) {
        return failTok(p, PE_OPTIONAL_IN_NEW, p.cur);
      }
      if (isSuper && !inChain && e.kind === E_SUPER) {
        return failTok(p, PE_SUPER_OPTIONAL, p.cur);
      }
      if (!bump(p)) {
        return null;
      }
      inChain = true;
      if (p.punct === P_LPAREN) {
        const args = parseArgs(p);
        if (args === null) {
          return null;
        }
        next = makeExprCall(e, args, true, s0, s0, l0, c0);
      } else if (p.punct === P_LBRACKET) {
        const pst = p.cur;
        if (!bump(p)) {
          return null;
        }
        const x = parseExpressionIn(p);
        if (x === null || !expect(p, P_RBRACKET)) {
          return null;
        }
        const prop = makePropComputed(x, t.start[pst], endOf(p, pst), t.line[pst], t.column[pst]);
        next = makeExprMember(e, prop, true, s0, s0, l0, c0);
      } else if (p.kind === TK_TEMPLATE) {
        return failTok(p, PE_TAGGED_OPTIONAL, p.cur);
      } else {
        const prop = parseMemberName(p);
        if (prop === null) {
          return null;
        }
        next = makeExprMember(e, prop, true, s0, s0, l0, c0);
      }
    } else if (p.punct === P_LBRACKET) {
      const pst = p.cur;
      if (!bump(p)) {
        return null;
      }
      const x = parseExpressionIn(p);
      if (x === null || !expect(p, P_RBRACKET)) {
        return null;
      }
      const prop = makePropComputed(x, t.start[pst], endOf(p, pst), t.line[pst], t.column[pst]);
      next = makeExprMember(e, prop, false, s0, s0, l0, c0);
    } else if (p.kind === TK_TEMPLATE) {
      if (inChain) {
        return failTok(p, PE_TAGGED_OPTIONAL, p.cur);
      }
      const q = parseTemplate(p, true);
      if (q === null) {
        return null;
      }
      next = makeExprTagged(e, q, s0, s0, l0, c0);
    } else if (p.punct === P_LPAREN && allowCall) {
      const args = parseArgs(p);
      if (args === null) {
        return null;
      }
      next = makeExprCall(e, args, false, s0, s0, l0, c0);
    } else {
      break;
    }
    if (!enter(p)) {
      return null;
    }
    links = links + 1;
    next.end = endFrom(p, s0);
    e = next;
  }
  p.depth = p.depth - links;
  if (p.depth < 0) {
    p.depth = 0;
  }
  if (inChain) {
    e = makeExprOptChain(e, e.start, e.end, e.line, e.col);
  }
  return e;
}

function parseMemberName(p) {
  const i = p.cur;
  if (p.kind === TK_IDENT) {
    const id = identOf(p, i);
    if (!bump(p)) {
      return null;
    }
    return makePropName(id, id.start, id.end, id.line, id.col);
  }
  if (p.kind === TK_PRIVATE) {
    const id = identOf(p, i);
    if (!referencePrivate(p.ee, id)) {
      return failNodeName(p, PE_PRIVATE_NOT_DEFINED, id, id.name);
    }
    if (!bump(p)) {
      return null;
    }
    return makePropPrivate(id, id.start, id.end, id.line, id.col);
  }
  return unexpected(p);
}

// `( ... )` of a call.
function parseArgs(p) {
  if (!expect(p, P_LPAREN)) {
    return null;
  }
  const savedIn = p.noIn;
  p.noIn = false;
  const args = argList(p);
  p.noIn = savedIn;
  return args;
}

function argList(p) {
  const t = p.t;
  const args = [];
  while (p.punct !== P_RPAREN) {
    if (p.punct === P_ELLIPSIS) {
      const s = p.cur;
      if (!bump(p)) {
        return null;
      }
      const a = parseAssignInner(p, false);
      if (a === null) {
        return null;
      }
      args.push(makeExprSpread(a, t.start[s], maxEnd(a, a), t.line[s], t.column[s]));
    } else {
      const a = parseAssignInner(p, false);
      if (a === null) {
        return null;
      }
      args.push(a);
    }
    const c = eat(p, P_COMMA);
    if (c < 0) {
      return null;
    }
    if (c === 0) {
      break;
    }
  }
  if (!expect(p, P_RPAREN)) {
    return null;
  }
  return args;
}

function parseImportExpr(p) {
  const t = p.t;
  const st = p.cur;
  if (!bump(p)) {
    return null;
  }
  const d = eat(p, P_DOT);
  if (d < 0) {
    return null;
  }
  if (d === 1) {
    if (p.word !== W_META) {
      return failTok(p, PE_EXPECTED_IMPORT_META, p.cur);
    }
    if (!bump(p)) {
      return null;
    }
    if (!p.isModule) {
      return failTok(p, PE_IMPORT_META_MODULE, st);
    }
    return makeExprImportMeta(t.start[st], endOf(p, st), t.line[st], t.column[st]);
  }
  if (p.punct !== P_LPAREN) {
    return failTok(p, PE_EXPECTED_IMPORT_PAREN, p.cur);
  }
  if (!bump(p)) {
    return null;
  }
  const savedIn = p.noIn;
  p.noIn = false;
  const source = parseAssignInner(p, false);
  let options = null;
  let ok = source !== null;
  if (ok) {
    const c = eat(p, P_COMMA);
    ok = c >= 0;
    if (c === 1 && p.punct !== P_RPAREN) {
      options = parseAssignInner(p, false);
      ok = options !== null && eat(p, P_COMMA) >= 0;
    }
  }
  ok = ok && expect(p, P_RPAREN);
  p.noIn = savedIn;
  if (!ok) {
    return null;
  }
  return makeExprImport(source, options, t.start[st], endOf(p, st), t.line[st], t.column[st]);
}

// ---- primary expressions ----

function parsePrimary(p) {
  const t = p.t;
  const st = p.cur;
  const k = p.kind;
  if (k === TK_IDENT) {
    const w = p.word;
    if (w === W_THIS) {
      if (!bump(p)) {
        return null;
      }
      return makeExprThis(t.start[st], t.end[st], t.line[st], t.column[st]);
    }
    if (w === W_NULL) {
      if (!bump(p)) {
        return null;
      }
      return makeExprNull(t.start[st], t.end[st], t.line[st], t.column[st]);
    }
    if (w === W_TRUE || w === W_FALSE) {
      if (!bump(p)) {
        return null;
      }
      return makeExprBool(w === W_TRUE, t.start[st], t.end[st], t.line[st], t.column[st]);
    }
    if (w === W_FUNCTION) {
      const f = parseFunction(p, st, false, false, false);
      if (f === null) {
        return null;
      }
      return makeExprFunction(f, t.start[st], endOf(p, st), t.line[st], t.column[st]);
    }
    if (w === W_CLASS) {
      const c = parseClass(p, false, false);
      if (c === null) {
        return null;
      }
      return makeExprClass(c, t.start[st], endOf(p, st), t.line[st], t.column[st]);
    }
    if (w === W_ASYNC) {
      const a = tryAsync(p, st);
      if (a !== null || p.err !== PE_NONE) {
        return a;
      }
    }
    const name = curName(p);
    const place = placeOf(p, st);
    const nw = nameWordOf(p, st);
    if (!checkIdentRef(p, nw, name, place)) {
      return null;
    }
    if (nw === W_ARGUMENTS && p.ctx.classInit) {
      return failTok(p, PE_ARGUMENTS_CLASS_INIT, st);
    }
    if (nw === W_AWAIT) {
      p.lastAwaitIdent = t.start[st];
    }
    if (!bump(p)) {
      return null;
    }
    if (p.punct === P_ARROW && !p.nl) {
      if (!checkBindingIdent(p, nw, name, place)) {
        return null;
      }
      return parseArrowRest(p, st, [patIdentAt(name, place)], false, t.start[st]);
    }
    return makeExprIdent(name, place.start, place.end, place.line, place.col);
  }
  if (k === TK_NUMBER) {
    if (!checkLegacy(p, st)) {
      return null;
    }
    const v = numberValue(t, st);
    if (!bump(p)) {
      return null;
    }
    return makeExprNum(v, t.start[st], t.end[st], t.line[st], t.column[st]);
  }
  if (k === TK_BIGINT) {
    const radix = bigintRadix(t, st);
    const digits = bigintDigits(t, st);
    if (!bump(p)) {
      return null;
    }
    return makeExprBigint(radix, digits, t.start[st], t.end[st], t.line[st], t.column[st]);
  }
  if (k === TK_STRING) {
    if (!checkLegacy(p, st)) {
      return null;
    }
    const v = stringValue(t, p.src, st);
    if (!bump(p)) {
      return null;
    }
    return makeExprStr(v, t.start[st], t.end[st], t.line[st], t.column[st]);
  }
  if (p.punct === P_SLASH || p.punct === P_SLASH_EQ) {
    dropPeek(p);
    const i = rescanRegex(p.lx, st);
    if (i < 0) {
      return failLex(p);
    }
    load(p, i);
    if (p.kind !== TK_REGEX) {
      return unexpected(p);
    }
    const pattern = regexPattern(t, p.src, i);
    const flags = regexFlags(t, p.src, i);
    if (!bump(p)) {
      return null;
    }
    return makeExprRegex(pattern, flags, t.start[i], endOf(p, i), t.line[i], t.column[i]);
  }
  return unexpected(p);
}

// `async function`, `async x => ...` and `async (...) => ...` (or a call
// of a function named `async`). Returns null with no error when `async`
// is a plain name here.
function tryAsync(p, st) {
  const t = p.t;
  const q = peek(p);
  if (q < 0) {
    return null;
  }
  if (tokNl(p, q)) {
    return null;
  }
  if (tokWord(p, q) === W_FUNCTION) {
    if (!bump(p)) {
      return null;
    }
    const f = parseFunction(p, st, true, false, false);
    if (f === null) {
      return null;
    }
    return makeExprFunction(f, t.start[st], endOf(p, st), t.line[st], t.column[st]);
  }
  if (t.kind[q] === TK_IDENT) {
    if (!bump(p)) {
      return null;
    }
    const i = p.cur;
    const name = curName(p);
    const place = placeOf(p, i);
    const w = nameWordOf(p, i);
    if (!bump(p)) {
      return null;
    }
    if (w === W_AWAIT) {
      return failTok(p, PE_AWAIT_PARAM_NAME, i);
    }
    const saved = p.ctx;
    p.ctx = copyCtx(saved);
    p.ctx.isAsync = true;
    const ok = checkBindingIdent(p, w, name, place);
    p.ctx = saved;
    if (!ok) {
      return null;
    }
    if (p.punct !== P_ARROW) {
      return failTok(p, PE_EXPECTED_ARROW_ASYNC_PARAM, p.cur);
    }
    return parseArrowRest(p, st, [patIdentAt(name, place)], true, place.start);
  }
  if (tokPunct(p, q) === P_LPAREN) {
    if (!bump(p)) {
      return null;
    }
    const pst = p.cur;
    if (!bump(p)) {
      return null;
    }
    const savedIn = p.noIn;
    p.noIn = false;
    const savedCover = p.coverErr;
    p.coverErr = null;
    const items = parseCoverList(p);
    p.noIn = savedIn;
    if (items === null) {
      return null;
    }
    if (p.punct === P_ARROW && !p.nl) {
      p.coverErr = null;
      const params = coverToParams(p, items);
      if (params === null) {
        return null;
      }
      const e = parseArrowRest(p, st, params, true, t.start[pst]);
      if (e === null) {
        return null;
      }
      p.coverErr = savedCover;
      return e;
    }
    if (p.coverErr !== null) {
      return raiseCover(p, p.coverErr);
    }
    p.coverErr = savedCover;
    const callee = makeExprIdent(tokenName(t, p.src, st), t.start[st], t.end[st], t.line[st], t.column[st]);
    return makeExprCall(callee, items, false, t.start[st], endOf(p, st), t.line[st], t.column[st]);
  }
  return null;
}

// After `(`: cover items up to and through `)`. A spread that is not last
// is marked as a bad rest. Sets p.coverTrailing for a trailing comma.
function parseCoverList(p) {
  const t = p.t;
  const items = [];
  let trailing = false;
  while (p.punct !== P_RPAREN) {
    if (p.punct === P_ELLIPSIS) {
      const s = p.cur;
      if (!bump(p)) {
        return null;
      }
      const a = parseAssignInner(p, true);
      if (a === null) {
        return null;
      }
      const spread = makeExprSpread(a, t.start[s], a.end, t.line[s], t.column[s]);
      items.push(spread);
      if (p.punct === P_RPAREN) {
        break;
      }
      markBadRest(p, t.start[s]);
    } else {
      const a = parseAssignInner(p, true);
      if (a === null) {
        return null;
      }
      items.push(a);
    }
    const c = eat(p, P_COMMA);
    if (c < 0) {
      return null;
    }
    if (c === 0) {
      break;
    }
    if (p.punct === P_RPAREN) {
      trailing = true;
    }
  }
  if (!expect(p, P_RPAREN)) {
    return null;
  }
  p.coverTrailing = trailing;
  return items;
}

function coverToParams(p, items) {
  const n = items.length;
  const params = [];
  let i = 0;
  while (i < n) {
    const item = items[i];
    if (item.kind === E_SPREAD) {
      if (i + 1 !== n || isBadRest(p, item.start)) {
        return failNode(p, PE_REST_PARAM_LAST, item);
      }
      if (item.arg.kind === E_ASSIGN) {
        return failNode(p, PE_REST_PARAM_DEFAULT, item);
      }
      const target = exprToPattern(p, item.arg, true);
      if (target === null) {
        return null;
      }
      params.push(makePatRest(target, item.start, item.end, item.line, item.col));
    } else {
      const x = exprToPattern(p, item, true);
      if (x === null) {
        return null;
      }
      params.push(x);
    }
    i = i + 1;
  }
  return params;
}

function parseParen(p) {
  const t = p.t;
  const st = p.cur;
  if (!bump(p)) {
    return null;
  }
  const savedIn = p.noIn;
  p.noIn = false;
  const items = parseCoverList(p);
  p.noIn = savedIn;
  if (items === null) {
    return null;
  }
  const trailing = p.coverTrailing;
  if (p.punct === P_ARROW && !p.nl) {
    p.coverErr = null;
    const params = coverToParams(p, items);
    if (params === null) {
      return null;
    }
    return parseArrowRest(p, st, params, false, t.start[st]);
  }
  if (p.coverErr !== null) {
    const e = p.coverErr;
    p.coverErr = null;
    return raiseCover(p, e);
  }
  if (items.length === 0 || trailing) {
    return failTok(p, PE_EXPECTED_ARROW_PARAMS, p.cur);
  }
  let i = 0;
  while (i < items.length) {
    if (items[i].kind === E_SPREAD) {
      return failNode(p, PE_PAREN_SPREAD, items[i]);
    }
    i = i + 1;
  }
  let inner = items[0];
  if (items.length > 1) {
    const last = items[items.length - 1];
    inner = makeExprSeq(items, inner.start, maxEnd(inner, last), inner.line, inner.col);
  }
  return makeExprParen(inner, t.start[st], endOf(p, st), t.line[st], t.column[st]);
}

function parseArray(p) {
  const t = p.t;
  const st = p.cur;
  if (!bump(p)) {
    return null;
  }
  const savedIn = p.noIn;
  p.noIn = false;
  const items = arrayItems(p);
  p.noIn = savedIn;
  if (items === null) {
    return null;
  }
  return makeExprArray(items, t.start[st], endOf(p, st), t.line[st], t.column[st]);
}

function arrayItems(p) {
  const t = p.t;
  const items = [];
  while (p.punct !== P_RBRACKET) {
    const c = eat(p, P_COMMA);
    if (c < 0) {
      return null;
    }
    if (c === 1) {
      items.push(null);
      continue;
    }
    if (p.punct === P_ELLIPSIS) {
      const s = p.cur;
      if (!bump(p)) {
        return null;
      }
      const a = parseAssignInner(p, true);
      if (a === null) {
        return null;
      }
      if (p.punct === P_COMMA) {
        markBadRest(p, t.start[s]);
      }
      items.push(makeExprSpread(a, t.start[s], a.end, t.line[s], t.column[s]));
    } else {
      const a = parseAssignInner(p, true);
      if (a === null) {
        return null;
      }
      items.push(a);
    }
    if (p.punct !== P_RBRACKET && !expect(p, P_COMMA)) {
      return null;
    }
  }
  if (!bump(p)) {
    return null;
  }
  return items;
}

function parseObject(p) {
  const t = p.t;
  const st = p.cur;
  if (!bump(p)) {
    return null;
  }
  const savedIn = p.noIn;
  p.noIn = false;
  const props = objectProps(p);
  p.noIn = savedIn;
  if (props === null) {
    return null;
  }
  return makeExprObject(props, t.start[st], endOf(p, st), t.line[st], t.column[st]);
}

function objectProps(p) {
  const props = [];
  // [whether a `__proto__: value` property was seen]
  const proto = [false];
  while (p.punct !== P_RBRACE) {
    const pr = parseObjectProp(p, proto);
    if (pr === null) {
      return null;
    }
    props.push(pr);
    if (p.punct !== P_RBRACE && !expect(p, P_COMMA)) {
      return null;
    }
  }
  if (!bump(p)) {
    return null;
  }
  return props;
}

function parseObjectProp(p, proto) {
  const t = p.t;
  const st = p.cur;
  if (p.punct === P_ELLIPSIS) {
    if (!bump(p)) {
      return null;
    }
    const a = parseAssignInner(p, true);
    if (a === null) {
      return null;
    }
    if (p.punct === P_COMMA) {
      markBadRest(p, t.start[st]);
    }
    return makeObjPropSpread(a, t.start[st], endOf(p, st), t.line[st], t.column[st]);
  }
  let isAsync = false;
  if (p.word === W_ASYNC) {
    const q = peek(p);
    if (q < 0) {
      return null;
    }
    if ((startsKey(p, q, false) || tokPunct(p, q) === P_STAR) && !tokNl(p, q)) {
      if (!bump(p)) {
        return null;
      }
      isAsync = true;
    }
  }
  const g = eat(p, P_STAR);
  if (g < 0) {
    return null;
  }
  const isGenerator = g === 1;
  let accessor = -1;
  if (!isAsync && !isGenerator && (p.word === W_GET || p.word === W_SET)) {
    const q = peek(p);
    if (q < 0) {
      return null;
    }
    if (startsKey(p, q, false)) {
      if (p.word === W_GET) {
        accessor = MK_GET;
      } else {
        accessor = MK_SET;
      }
      if (!bump(p)) {
        return null;
      }
    }
  }
  const kst = p.cur;
  const key = parsePropKey(p, false);
  if (key === null) {
    return null;
  }
  if (isAsync || isGenerator || accessor >= 0 || p.punct === P_LPAREN) {
    let kind = MK_METHOD;
    let flavor = FL_METHOD;
    if (accessor === MK_GET) {
      kind = MK_GET;
      flavor = FL_GETTER;
    } else if (accessor === MK_SET) {
      kind = MK_SET;
      flavor = FL_SETTER;
    }
    const f = parseFunctionRest(p, p.cur, null, isAsync, isGenerator, flavor);
    if (f === null) {
      return null;
    }
    return makeObjPropMethod(key, kind, f, t.start[st], endOf(p, st), t.line[st], t.column[st]);
  }
  const c = eat(p, P_COLON);
  if (c < 0) {
    return null;
  }
  if (c === 1) {
    if (key.kind !== PK_COMPUTED && keyWord(key) === W_PROTO) {
      if (proto[0] && p.coverErr === null) {
        p.coverErr = coverError(PE_DUPLICATE_PROTO, p, st);
      }
      proto[0] = true;
    }
    const value = parseAssignInner(p, true);
    if (value === null) {
      return null;
    }
    return makeObjPropInit(key, value, t.start[st], endOf(p, st), t.line[st], t.column[st]);
  }
  if (key.kind !== PK_IDENT) {
    return failTok(p, PE_EXPECTED_COLON, p.cur);
  }
  const id = key.ident;
  const w = unitsWord(id.name);
  if (tokEscaped(p, kst) && isReservedWord(w)) {
    return failNode(p, PE_KEYWORD_ESCAPES, id);
  }
  if (!checkIdentRef(p, w, id.name, id)) {
    return null;
  }
  if (w === W_ARGUMENTS && p.ctx.classInit) {
    return failNode(p, PE_ARGUMENTS_CLASS_INIT, id);
  }
  if (w === W_AWAIT) {
    p.lastAwaitIdent = id.start;
  }
  if (p.punct === P_EQ) {
    const eq = p.cur;
    if (!bump(p)) {
      return null;
    }
    const d = parseAssignInner(p, false);
    if (d === null) {
      return null;
    }
    if (p.coverErr === null) {
      p.coverErr = coverError(PE_COVER_INITIALIZER, p, eq);
    }
    const target = patIdentAt(id.name, id);
    const value = makeExprAssign(A_ASSIGN, target, d, id.start, maxEnd(id, d), id.line, id.col);
    return makeObjPropInit(key, value, t.start[st], endOf(p, st), t.line[st], t.column[st]);
  }
  return makeObjPropShorthand(id, t.start[st], endOf(p, st), t.line[st], t.column[st]);
}

// A template from its first piece (the current token) on.
function parseTemplate(p, tagged) {
  const t = p.t;
  const first = p.cur;
  const quasis = [];
  const exprs = [];
  while (true) {
    const i = p.cur;
    if (p.kind !== TK_TEMPLATE) {
      return unexpected(p);
    }
    const cooked = templateCooked(t, p.src, i);
    if (!tagged && cooked === null) {
      const at = templateBadEscape(t, i);
      const li = lineIndex(p.lx.starts, at);
      return fail(p, PE_TEMPLATE_ESCAPE, at, li + 1, at - p.lx.starts[li] + 1);
    }
    const raw = templateRaw(t, p.src, i);
    quasis.push(makeTemplateElement(cooked, raw, t.start[i], t.end[i], t.line[i], t.column[i]));
    const tail = (t.flags[i] & F_TAIL) !== 0;
    if (!bump(p)) {
      return null;
    }
    if (tail) {
      break;
    }
    const savedIn = p.noIn;
    p.noIn = false;
    const e = parseExpression(p);
    p.noIn = savedIn;
    if (e === null) {
      return null;
    }
    exprs.push(e);
    if (p.punct !== P_RBRACE) {
      return failTok(p, PE_TEMPLATE_CLOSE, p.cur);
    }
    dropPeek(p);
    const k = rescanTemplate(p.lx, p.cur);
    if (k < 0) {
      return failLex(p);
    }
    load(p, k);
  }
  return makeTemplate(quasis, exprs, t.start[first], endOf(p, first), t.line[first], t.column[first]);
}

// ---- modules ----

// `what`: W_IMPORT or W_EXPORT, for the message.
function requireModuleTop(p, what, st) {
  let code = PE_NONE;
  if (!p.isModule) {
    code = PE_MODULE_ONLY;
  } else if (p.scopeDepth !== 1 || p.ctx.inFunction) {
    code = PE_MODULE_TOP;
  }
  if (code === PE_NONE) {
    return true;
  }
  if (p.err === PE_NONE) {
    p.errArg = what;
  }
  failTok(p, code, st);
  return false;
}

function parseStringLit(p) {
  const t = p.t;
  const i = p.cur;
  if (p.kind !== TK_STRING) {
    return failCur(p, PE_EXPECTED_STRING);
  }
  if (!checkLegacy(p, i)) {
    return null;
  }
  const v = stringValue(t, p.src, i);
  if (!bump(p)) {
    return null;
  }
  return makeStrLit(v, t.start[i], t.end[i], t.line[i], t.column[i]);
}

// Whether code units form well-formed UTF-16 (no lone surrogates).
function wellFormed(units) {
  let i = 0;
  while (i < units.length) {
    const u = units[i];
    if (u >= 0xd800 && u <= 0xdbff) {
      if (i + 1 >= units.length || units[i + 1] < 0xdc00 || units[i + 1] > 0xdfff) {
        return false;
      }
      i = i + 2;
    } else if (u >= 0xdc00 && u <= 0xdfff) {
      return false;
    } else {
      i = i + 1;
    }
  }
  return true;
}

function parseModuleName(p) {
  if (p.kind === TK_STRING) {
    const s = parseStringLit(p);
    if (s === null) {
      return null;
    }
    if (!wellFormed(s.value)) {
      return failNode(p, PE_EXPORT_NAME_UNICODE, s);
    }
    return makeModuleNameStr(s, s.start, s.end, s.line, s.col);
  }
  if (p.kind === TK_IDENT) {
    const id = identOf(p, p.cur);
    if (!bump(p)) {
      return null;
    }
    return makeModuleNameIdent(id, id.start, id.end, id.line, id.col);
  }
  return unexpected(p);
}

function moduleNameText(m) {
  if (m.kind === MN_IDENT) {
    return m.ident.name;
  }
  return m.str.value;
}

function moduleNameNode(m) {
  if (m.kind === MN_IDENT) {
    return m.ident;
  }
  return m.str;
}

// `from "source"` and any attributes. Returns [source, attributes].
function parseFromClause(p) {
  if (!expectWord(p, W_FROM)) {
    return null;
  }
  const source = parseStringLit(p);
  if (source === null) {
    return null;
  }
  const attrs = parseImportAttributes(p);
  if (attrs === null) {
    return null;
  }
  return [source, attrs];
}

function parseImportAttributes(p) {
  const attrs = [];
  if (p.word !== W_WITH) {
    return attrs;
  }
  if (!bump(p) || !expect(p, P_LBRACE)) {
    return null;
  }
  const seen = newNameSet();
  while (p.punct !== P_RBRACE) {
    let key = null;
    if (p.kind === TK_STRING) {
      const s = parseStringLit(p);
      if (s === null) {
        return null;
      }
      key = makeModuleNameStr(s, s.start, s.end, s.line, s.col);
    } else if (p.kind === TK_IDENT) {
      const id = identOf(p, p.cur);
      if (!bump(p)) {
        return null;
      }
      key = makeModuleNameIdent(id, id.start, id.end, id.line, id.col);
    } else {
      return unexpected(p);
    }
    const text = moduleNameText(key);
    if (!nameSetAdd(seen, text)) {
      return failNodeName(p, PE_DUPLICATE_ATTRIBUTE, moduleNameNode(key), text);
    }
    if (!expect(p, P_COLON)) {
      return null;
    }
    const value = parseStringLit(p);
    if (value === null) {
      return null;
    }
    attrs.push(makeImportAttribute(key, value, key.start, value.end, key.line, key.col));
    if (p.punct !== P_RBRACE && !expect(p, P_COMMA)) {
      return null;
    }
  }
  if (!bump(p)) {
    return null;
  }
  return attrs;
}

function importBinding(p) {
  const i = p.cur;
  if (p.kind !== TK_IDENT) {
    return unexpected(p);
  }
  const id = identOf(p, i);
  if (!checkBindingIdent(p, nameWordOf(p, i), id.name, id)) {
    return null;
  }
  if (!bump(p)) {
    return null;
  }
  if (!lexicalName(p, id)) {
    return null;
  }
  return id;
}

function parseImport(p, st) {
  const t = p.t;
  if (!requireModuleTop(p, W_IMPORT, st) || !bump(p)) {
    return null;
  }
  const specifiers = [];
  if (p.kind === TK_STRING) {
    const source = parseStringLit(p);
    if (source === null) {
      return null;
    }
    const attributes = parseImportAttributes(p);
    if (attributes === null || !semicolon(p)) {
      return null;
    }
    const d = makeImportDecl(specifiers, source, attributes, t.start[st], endOf(p, st), t.line[st], t.column[st]);
    return makeStmtImport(d, t.start[st], endOf(p, st), t.line[st], t.column[st]);
  }
  let needMore = true;
  let hasDefault = p.kind === TK_IDENT && p.word !== W_FROM;
  if (!hasDefault && p.word === W_FROM) {
    const q = peek(p);
    if (q < 0) {
      return null;
    }
    hasDefault = tokWord(p, q) === W_FROM;
  }
  if (hasDefault) {
    const id = importBinding(p);
    if (id === null) {
      return null;
    }
    specifiers.push(makeImportSpecDefault(id, id.start, id.end, id.line, id.col));
    const c = eat(p, P_COMMA);
    if (c < 0) {
      return null;
    }
    needMore = c === 1;
  }
  if (needMore) {
    const star = eat(p, P_STAR);
    if (star < 0) {
      return null;
    }
    if (star === 1) {
      if (!expectWord(p, W_AS)) {
        return null;
      }
      const id = importBinding(p);
      if (id === null) {
        return null;
      }
      specifiers.push(makeImportSpecNamespace(id, id.start, id.end, id.line, id.col));
    } else {
      const brace = eat(p, P_LBRACE);
      if (brace < 0) {
        return null;
      }
      if (brace === 0) {
        return unexpected(p);
      }
      while (p.punct !== P_RBRACE) {
        const imported = parseModuleName(p);
        if (imported === null) {
          return null;
        }
        let local = null;
        if (p.word === W_AS) {
          if (!bump(p)) {
            return null;
          }
          local = importBinding(p);
          if (local === null) {
            return null;
          }
        } else if (imported.kind === MN_IDENT) {
          local = imported.ident;
          if (!checkBindingIdent(p, unitsWord(local.name), local.name, local) || !lexicalName(p, local)) {
            return null;
          }
        } else {
          return failNode(p, PE_STRING_IMPORT_AS, imported.str);
        }
        specifiers.push(makeImportSpecNamed(imported, local, imported.start, local.end, imported.line, imported.col));
        if (p.punct !== P_RBRACE && !expect(p, P_COMMA)) {
          return null;
        }
      }
      if (!bump(p)) {
        return null;
      }
    }
  }
  const from = parseFromClause(p);
  if (from === null || !semicolon(p)) {
    return null;
  }
  const d = makeImportDecl(specifiers, from[0], from[1], t.start[st], endOf(p, st), t.line[st], t.column[st]);
  return makeStmtImport(d, t.start[st], endOf(p, st), t.line[st], t.column[st]);
}

const DEFAULT_UNITS = [0x64, 0x65, 0x66, 0x61, 0x75, 0x6c, 0x74];

// The names an exported declaration binds, as exports.
function exportBound(p, stmt) {
  if (stmt.kind === S_VAR) {
    const ds = stmt.decl.decls;
    let i = 0;
    while (i < ds.length) {
      const names = [];
      boundNames(ds[i].target, names);
      let j = 0;
      while (j < names.length) {
        if (!exportIdent(p, names[j])) {
          return false;
        }
        j = j + 1;
      }
      i = i + 1;
    }
    return true;
  }
  if (stmt.kind === S_FUNCTION) {
    return exportIdent(p, stmt.func.id);
  }
  return exportIdent(p, stmt.cls.id);
}

function parseExport(p, st) {
  const t = p.t;
  if (!requireModuleTop(p, W_EXPORT, st) || !bump(p)) {
    return null;
  }
  let decl = null;
  const star = eat(p, P_STAR);
  if (star < 0) {
    return null;
  }
  if (star === 1) {
    let exported = null;
    if (p.word === W_AS) {
      if (!bump(p)) {
        return null;
      }
      exported = parseModuleName(p);
      if (exported === null || !exportName(p, exported)) {
        return null;
      }
    }
    const from = parseFromClause(p);
    if (from === null || !semicolon(p)) {
      return null;
    }
    decl = makeExportAll(exported, from[0], from[1], t.start[st], endOf(p, st), t.line[st], t.column[st]);
  } else if (p.punct === P_LBRACE) {
    if (!bump(p)) {
      return null;
    }
    const specs = [];
    while (p.punct !== P_RBRACE) {
      const local = parseModuleName(p);
      if (local === null) {
        return null;
      }
      let exported = local;
      if (p.word === W_AS) {
        if (!bump(p)) {
          return null;
        }
        exported = parseModuleName(p);
        if (exported === null) {
          return null;
        }
      }
      if (!exportName(p, exported)) {
        return null;
      }
      specs.push(makeExportSpec(local, exported, local.start, exported.end, local.line, local.col));
      if (p.punct !== P_RBRACE && !expect(p, P_COMMA)) {
        return null;
      }
    }
    if (!bump(p)) {
      return null;
    }
    let source = null;
    let attributes = [];
    if (p.word === W_FROM) {
      const from = parseFromClause(p);
      if (from === null) {
        return null;
      }
      source = from[0];
      attributes = from[1];
    } else {
      let i = 0;
      while (i < specs.length) {
        const local = specs[i].local;
        if (local.kind === MN_STR) {
          return failNode(p, PE_STRING_EXPORT_FROM, local.str);
        }
        const w = unitsWord(local.ident.name);
        if (isReservedWord(w) || isStrictReservedWord(w) || w === W_AWAIT) {
          return failNodeName(p, PE_RESERVED_WORD, local.ident, local.ident.name);
        }
        exportLocal(p.ee, local.ident);
        i = i + 1;
      }
    }
    if (!semicolon(p)) {
      return null;
    }
    decl = makeExportNamed(specs, source, attributes, t.start[st], endOf(p, st), t.line[st], t.column[st]);
  } else if (p.word === W_DEFAULT) {
    const dt = p.cur;
    if (!bump(p)) {
      return null;
    }
    if (!addExport(p.ee, DEFAULT_UNITS)) {
      return failTokName(p, PE_EXPORTED_TWICE, dt, DEFAULT_UNITS);
    }
    const fst = p.cur;
    let def = null;
    let isAsyncFunction = false;
    if (p.word === W_ASYNC) {
      const q = peek(p);
      if (q < 0) {
        return null;
      }
      isAsyncFunction = tokWord(p, q) === W_FUNCTION && !tokNl(p, q);
    }
    if (p.word === W_FUNCTION || isAsyncFunction) {
      if (isAsyncFunction && !bump(p)) {
        return null;
      }
      const f = parseFunction(p, fst, isAsyncFunction, true, true);
      if (f === null) {
        return null;
      }
      def = makeExportDefaultFunction(f, f.start, f.end, f.line, f.col);
    } else if (p.word === W_CLASS) {
      const c = parseClass(p, true, true);
      if (c === null || (c.id !== null && !lexicalName(p, c.id))) {
        return null;
      }
      def = makeExportDefaultClass(c, c.start, c.end, c.line, c.col);
    } else {
      const e = parseAssignIn(p);
      if (e === null || !semicolon(p)) {
        return null;
      }
      def = makeExportDefaultExpr(e, e.start, e.end, e.line, e.col);
    }
    decl = makeExportDefault(def, t.start[st], endOf(p, st), t.line[st], t.column[st]);
  } else {
    const dst = p.cur;
    const w = p.word;
    let stmt = null;
    if (w === W_VAR) {
      const d = parseVarDecl(p, VK_VAR, false);
      if (d === null || !semicolon(p)) {
        return null;
      }
      stmt = makeStmtVar(d, t.start[dst], endOf(p, dst), t.line[dst], t.column[dst]);
    } else if (w === W_LET || w === W_CONST || w === W_FUNCTION || w === W_CLASS || w === W_ASYNC) {
      stmt = declarationOrStatement(p);
      if (stmt === null) {
        return null;
      }
    } else {
      return unexpected(p);
    }
    const k = stmt.kind;
    if (k !== S_VAR && k !== S_FUNCTION && k !== S_CLASS) {
      return failTok(p, PE_EXPECTED_DECLARATION, dst);
    }
    if (!exportBound(p, stmt)) {
      return null;
    }
    decl = makeExportDecl(stmt, stmt.start, stmt.end, stmt.line, stmt.col);
  }
  return makeStmtExport(decl, t.start[st], endOf(p, st), t.line[st], t.column[st]);
}
