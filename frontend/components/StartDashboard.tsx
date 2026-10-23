// Flux Browser Built-In Home Screen Component (flux://home)
// Author: Buntos

export interface StartDashboardProps {
  version: string;
  defaultQuery: string;
  activeEngineName: string;
  activeEngineLogo: string;
  searchPlaceholder: string;
}

export function StartDashboard(props: StartDashboardProps) {
  return (
    <div className="home-blank">
      <div className="home-brand-row">
        <img id="flux-brand-logo" src="flux://logo.png" width="68" height="68" alt="Flux" />
        <h1 className="home-title">Flux</h1>
      </div>

      <div className="home-search-wrap">
        <form className="search-row" action="flux://navigate" method="GET">
          <img
            id="home-engine-badge"
            src={props.activeEngineLogo}
            width="28"
            height="28"
            alt={props.activeEngineName}
          />
          <input
            id="home-q"
            type="text"
            name="q"
            value={props.defaultQuery}
            placeholder={props.searchPlaceholder}
          />
          <button id="home-go" type="submit">
            Search
          </button>
        </form>
      </div>
    </div>
  );
}
