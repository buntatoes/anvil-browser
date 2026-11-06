// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
// Stored script text for anvil://settings. frontend.rs copies this file into
// a script element after anvil_core.js.
// Author: Buntato

let currentSettingsSection = 'all';

function filterSettingsCards(query) {
  let needle = (query || '').trim().toLowerCase();
  let cards = document.querySelectorAll('.settings-card');
  cards.forEach((card) => {
    let group = card.getAttribute('data-section');
    let text = (card.textContent || '').toLowerCase();
    let inSection = currentSettingsSection === 'all' || group === currentSettingsSection;
    let matches = needle === '' || text.indexOf(needle) !== -1;
    if (needle !== '') {
      card.style.display = matches ? 'block' : 'none';
    } else {
      card.style.display = inSection ? 'block' : 'none';
    }
  });
}

function switchSettingsSection(sectionName) {
  let cards = document.querySelectorAll('.settings-card');
  cards.forEach((card) => {
    let group = card.getAttribute('data-section');
    if (sectionName === 'all' || group === sectionName) {
      card.style.display = 'block';
    } else {
      card.style.display = 'none';
    }
  });
  currentSettingsSection = sectionName;
  writeAnvilPref('anvil.settings.section', sectionName);
  let search = document.getElementById('settings-card-search');
  if (search && search.value) {
    filterSettingsCards(search.value);
  }
  let navIds = [
    'nav-sec-all',
    'nav-sec-appearance',
    'nav-sec-search',
    'nav-sec-js',
    'nav-sec-privacy',
    'nav-sec-system'
  ];
  let targetNavId = 'nav-sec-all';
  if (sectionName === 'appearance') {
    targetNavId = 'nav-sec-appearance';
  } else if (sectionName === 'search') {
    targetNavId = 'nav-sec-search';
  } else if (sectionName === 'js') {
    targetNavId = 'nav-sec-js';
  } else if (sectionName === 'privacy') {
    targetNavId = 'nav-sec-privacy';
  } else if (sectionName === 'system') {
    targetNavId = 'nav-sec-system';
  }
  highlightActiveNavItem(targetNavId, navIds);
}

function restoreInitialSettingsSection() {
  let href = window.location.href;
  let querySec = parseAnvilQueryParam(href, 'section');
  if (querySec !== '') {
    switchSettingsSection(querySec);
    return querySec;
  }
  return 'all';
}

function initAnvilSettings() {
  recordAnvilPageVisit('settings');

  let navAll = document.getElementById('nav-sec-all');
  let navAppearance = document.getElementById('nav-sec-appearance');
  let navSearch = document.getElementById('nav-sec-search');
  let navPrivacy = document.getElementById('nav-sec-privacy');
  let navJs = document.getElementById('nav-sec-js');
  let navSystem = document.getElementById('nav-sec-system');

  if (navAll) {
    navAll.addEventListener('click', (e) => {
      if (e && e.preventDefault) { e.preventDefault(); }
      switchSettingsSection('all');
    });
  }
  if (navAppearance) {
    navAppearance.addEventListener('click', (e) => {
      if (e && e.preventDefault) { e.preventDefault(); }
      switchSettingsSection('appearance');
    });
  }
  if (navSearch) {
    navSearch.addEventListener('click', (e) => {
      if (e && e.preventDefault) { e.preventDefault(); }
      switchSettingsSection('search');
    });
  }
  if (navPrivacy) {
    navPrivacy.addEventListener('click', (e) => {
      if (e && e.preventDefault) { e.preventDefault(); }
      switchSettingsSection('privacy');
    });
  }
  if (navJs) {
    navJs.addEventListener('click', (e) => {
      if (e && e.preventDefault) { e.preventDefault(); }
      switchSettingsSection('js');
    });
  }
  if (navSystem) {
    navSystem.addEventListener('click', (e) => {
      if (e && e.preventDefault) { e.preventDefault(); }
      switchSettingsSection('system');
    });
  }

  restoreInitialSettingsSection();
  let search = document.getElementById('settings-card-search');
  if (search) {
    search.addEventListener('input', () => {
      filterSettingsCards(search.value);
    });
  }
}

initAnvilSettings();
