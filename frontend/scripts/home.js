// Flux Browser Home Page Interactive Controller (flux://home)
// Author: Buntos

function initFluxHome() {
  let searchInput = document.getElementById('home-q');
  let engineBadge = document.getElementById('home-engine-badge');
  if (searchInput) {
    searchInput.focus();
  }
  if (engineBadge) {
    engineBadge.addEventListener('click', () => {
      window.location.href = 'flux://settings?section=search';
    });
  }
}

initFluxHome();
