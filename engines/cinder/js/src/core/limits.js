// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Every limit the engine checks, with the reference front end's defaults
// (its Limits::default(), MAX_COMPILE_DEPTH and parser defaults). Names in
// LIMITS match what `refdump --limits` prints; a unit test and the check
// script compare the two.

// Parser.
export const MAX_SOURCE_BYTES = 67108864;
export const MAX_PARSE_DEPTH = 1000;

// Compiler.
export const MAX_COMPILE_DEPTH_CAP = 100000;
export const MAX_COMPILE_DEPTH = 2500;
export const MAX_REGISTERS_PER_FUNCTION = 65535;

// Heap.
export const MAX_HEAP_BYTES = 1073741824;
export const INITIAL_GC_TRIGGER = 8388608;
export const GC_GROWTH_PERCENT = 200;

// Running.
export const MAX_STEPS = 1000000000;
export const INTERRUPT_CHECK_INTERVAL = 4096;
export const MAX_CALL_DEPTH = 10000;
export const MAX_NATIVE_REENTRY = 256;

// Values.
export const MAX_STRING_LENGTH = 1073741823;
export const ROPE_MIN_LENGTH = 64;
export const DICTIONARY_THRESHOLD = 64;
export const MAX_TRANSITIONS_PER_SHAPE = 32;
export const MAX_JSON_DEPTH = 10000;
export const MAX_JOIN_DEPTH = 10000;
export const MAX_STACK_LINES = 64;

// The language's own array length limit, 2^32 - 1.
export const MAX_ARRAY_LENGTH = 4294967295;

// Name and value of each limit, in the order `refdump --limits` prints.
export const LIMITS = [
  ["parser.max_source_bytes", MAX_SOURCE_BYTES],
  ["parser.max_depth", MAX_PARSE_DEPTH],
  ["limits.MAX_COMPILE_DEPTH", MAX_COMPILE_DEPTH_CAP],
  ["limits.max_heap_bytes", MAX_HEAP_BYTES],
  ["limits.initial_gc_trigger", INITIAL_GC_TRIGGER],
  ["limits.gc_growth_percent", GC_GROWTH_PERCENT],
  ["limits.max_steps", MAX_STEPS],
  ["limits.interrupt_check_interval", INTERRUPT_CHECK_INTERVAL],
  ["limits.max_call_depth", MAX_CALL_DEPTH],
  ["limits.max_native_reentry", MAX_NATIVE_REENTRY],
  ["limits.max_string_length", MAX_STRING_LENGTH],
  ["limits.rope_min_length", ROPE_MIN_LENGTH],
  ["limits.dictionary_threshold", DICTIONARY_THRESHOLD],
  ["limits.max_transitions_per_shape", MAX_TRANSITIONS_PER_SHAPE],
  ["limits.max_source_bytes", MAX_SOURCE_BYTES],
  ["limits.max_compile_depth", MAX_COMPILE_DEPTH],
  ["limits.max_registers_per_function", MAX_REGISTERS_PER_FUNCTION],
  ["limits.max_json_depth", MAX_JSON_DEPTH],
  ["limits.max_join_depth", MAX_JOIN_DEPTH],
  ["limits.max_stack_lines", MAX_STACK_LINES],
];

// Value of a limit by its LIMITS name, or -1 when there is none.
export function limitValue(name) {
  for (let i = 0; i < LIMITS.length; i++) {
    if (LIMITS[i][0] === name) {
      return LIMITS[i][1];
    }
  }
  return -1;
}
