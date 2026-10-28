// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Stored script text for anvil://history. frontend.rs copies this file into
// a script element after anvil_core.js.
// Author: Buntato

function scanHistoryEntries() {
  let items = document.querySelectorAll('.history-item');
  writeAnvilSession('anvil.history.visible_items', String(items.length));
  return items.length;
}

function filterHistoryEntries(rawQuery) {
  let needle = String(rawQuery || '').toLowerCase().trim();
  let items = document.querySelectorAll('.history-item');
  let visible = 0;
  items.forEach((item) => {
    let text = String(item.textContent || '').toLowerCase();
    if (needle === '' || text.indexOf(needle) !== -1) {
      item.style.display = 'block';
      visible = visible + 1;
    } else {
      item.style.display = 'none';
    }
  });
  writeAnvilSession('anvil.history.visible_items', String(visible));
  return visible;
}

function bindHistoryEntries() {
  let items = document.querySelectorAll('.history-item');
  let index = 0;
  items.forEach((item) => {
    let slot = index;
    item.addEventListener('click', () => {
      writeAnvilSession('anvil.history.selected_item', String(slot));
    });
    index = index + 1;
  });
  let filterInput = document.getElementById('history-filter');
  if (filterInput) {
    filterInput.addEventListener('input', () => {
      filterHistoryEntries(filterInput.value);
    });
  }
  return index;
}

function initAnvilHistory() {
  recordAnvilPageVisit('history');
  scanHistoryEntries();
  bindHistoryEntries();
}

initAnvilHistory();
