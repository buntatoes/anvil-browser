// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato. Verify canonical repository history at https://github.com/buntatoes

package dom

import (
	"testing"
)

func TestParseHTMLAndRecovery(t *testing.T) {
	doc := ParseHTML(`<!DOCTYPE html>
<html>
<head><title>Forge Test</title><style>body { color: #111; }</style></head>
<body>
  <div id="main" class="card primary" data-role="hero">
    <p>First paragraph<div>Auto-closed paragraph</div>
    <b><i>Misnested</b>Still italic</i>
    <table><span>Fostered span</span><tr><td>Cell 1<td>Cell 2</table>
  </div>
</body>
</html>`)

	if got := doc.Title(); got != "Forge Test" {
		t.Fatalf("expected title 'Forge Test', got %q", got)
	}
	if len(doc.StyleSheets()) != 1 {
		t.Fatalf("expected 1 stylesheet, got %d", len(doc.StyleSheets()))
	}
	mainID, ok := doc.GetElementByID("main")
	if !ok {
		t.Fatal("expected #main element")
	}
	if !doc.MatchesSelector(mainID, "div#main.card.primary[data-role^=her]") {
		t.Fatal("expected #main to match compound selector")
	}
	cells := doc.QuerySelectorAll("table td")
	if len(cells) != 2 {
		t.Fatalf("expected 2 table cells, got %d", len(cells))
	}
	fostered := doc.QuerySelectorAll("#main > span")
	if len(fostered) != 1 {
		t.Fatalf("expected 1 fostered span before table, got %d", len(fostered))
	}
}

func TestSelectorsLevel4AndShadowDOM(t *testing.T) {
	doc := ParseHTML(`<ul>
		<li class="item">One</li>
		<li class="item active"><a href="#sec">Two</a></li>
		<li class="item">Three</li>
	</ul>
	<div id="host"><span slot="title">Projected Header</span></div>`)

	items := doc.QuerySelectorAll("ul > li.item:nth-child(2n+1)")
	if len(items) != 2 {
		t.Fatalf("expected 2 odd items, got %d", len(items))
	}
	hasLink := doc.QuerySelectorAll("li:has(a[href])")
	if len(hasLink) != 1 {
		t.Fatalf("expected 1 li:has(a[href]), got %d", len(hasLink))
	}
	host, ok := doc.GetElementByID("host")
	if !ok {
		t.Fatal("missing #host")
	}
	sr, err := doc.AttachShadow(host, true)
	if err != nil {
		t.Fatal(err)
	}
	slotEl := doc.CreateElement("slot")
	doc.Node(slotEl).Data.SetAttr("name", "title")
	_ = doc.Append(sr, slotEl)

	composed := doc.ComposedChildren(host)
	if len(composed) != 1 || doc.TextContent(composed[0]) != "Projected Header" {
		t.Fatalf("unexpected composed children: %v", composed)
	}
}

func TestDialogFormMethodSubmissionAndOverride(t *testing.T) {
	doc := ParseHTML(`<!DOCTYPE html>
<html><body>
  <dialog id="dlg" open data-modal="true">
    <form id="f" method="dialog" action="https://example.com/never">
      <input id="req" type="email" required value="not-an-email">
      <button id="confirm" type="submit" value="accepted">Confirm</button>
      <button id="plain" type="submit">Plain</button>
    </form>
  </dialog>
  <dialog id="dlg2" open data-return-value="kept">
    <form id="f2" method="post" action="https://example.com/post">
      <button id="cancel" type="submit" formmethod="dialog">Cancel</button>
    </form>
  </dialog>
</body></html>`)

	dlg, _ := doc.GetElementByID("dlg")
	confirmBtn, _ := doc.GetElementByID("confirm")
	if !doc.IsDialogOpen(dlg) {
		t.Fatal("expected #dlg to start open")
	}
	if req, ok := doc.BuildFormSubmission("https://example.com/", confirmBtn); ok || req != nil {
		t.Fatalf("expected dialog submission to perform zero navigation, got %+v", req)
	}
	if doc.IsDialogOpen(dlg) {
		t.Fatal("expected #dlg to close after method=dialog submission")
	}
	if got := doc.DialogReturnValue(dlg); got != "accepted" {
		t.Fatalf("expected returnValue 'accepted', got %q", got)
	}

	dlg2, _ := doc.GetElementByID("dlg2")
	cancelBtn, _ := doc.GetElementByID("cancel")
	dlgID, closed, ok := doc.SubmitDialogForm(cancelBtn)
	if !ok || !closed || dlgID != dlg2 {
		t.Fatalf("expected formmethod=dialog override to close #dlg2, got (%d, %v, %v)", dlgID, closed, ok)
	}
	if doc.IsDialogOpen(dlg2) {
		t.Fatal("expected #dlg2 to be closed")
	}
	if got := doc.DialogReturnValue(dlg2); got != "kept" {
		t.Fatalf("expected missing value attribute to preserve returnValue 'kept', got %q", got)
	}
}
