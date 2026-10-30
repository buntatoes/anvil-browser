// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
var a = function self() { return self; };
var b = function self() { var self; return self; };
var c = function self(self) { return self; };
var d = function self() { { function self() {} } return self; };
