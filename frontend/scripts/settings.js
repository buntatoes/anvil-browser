// Flux Browser Two-Column Settings Controller (flux://settings)
// Author: Buntos

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
}

function initFluxSettings() {
  let navAll = document.getElementById('nav-sec-all');
  let navSearch = document.getElementById('nav-sec-search');
  let navPrivacy = document.getElementById('nav-sec-privacy');
  let navJs = document.getElementById('nav-sec-js');
  let navSystem = document.getElementById('nav-sec-system');

  if (navAll) {
    navAll.addEventListener('click', () => {
      switchSettingsSection('all');
    });
  }
  if (navSearch) {
    navSearch.addEventListener('click', () => {
      switchSettingsSection('search');
    });
  }
  if (navPrivacy) {
    navPrivacy.addEventListener('click', () => {
      switchSettingsSection('privacy');
    });
  }
  if (navJs) {
    navJs.addEventListener('click', () => {
      switchSettingsSection('js');
    });
  }
  if (navSystem) {
    navSystem.addEventListener('click', () => {
      switchSettingsSection('system');
    });
  }
}

initFluxSettings();
