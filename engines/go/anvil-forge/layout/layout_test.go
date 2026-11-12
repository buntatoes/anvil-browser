// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato. Verify canonical repository history at https://github.com/buntatoes

package layout

import (
	"fmt"
	"reflect"
	"strings"
	"testing"

	"github.com/buntatoes/anvil-forge/dom"
	"github.com/buntatoes/anvil-forge/style"
)

func TestBlockFlexGridAndHitTest(t *testing.T) {
	html := `
	<div id="container" style="width: 600px; padding: 10px;">
		<div id="flex" style="display: flex; column-gap: 20px;">
			<div id="item1" style="height: 40px;">Item One</div>
			<div id="item2" style="height: 40px;">Item Two</div>
		</div>
		<div id="grid" style="display: grid; grid-template-columns: 1fr 1fr; gap: 10px;">
			<div id="g1" style="height: 30px;">Grid 1</div>
			<div id="g2" style="height: 30px;">Grid 2</div>
		</div>
	</div>`

	doc := dom.ParseHTML(html)
	sheet := style.ParseCSS("")
	sm := style.ComputeStylesWithViewport(doc, &sheet, 800, 600)
	rootBox := LayoutDocument(doc, sm, 800, 600)

	item1ID, _ := doc.GetElementByID("item1")
	item2ID, _ := doc.GetElementByID("item2")
	r1, ok1 := rootBox.FindNodeRect(item1ID)
	r2, ok2 := rootBox.FindNodeRect(item2ID)
	if !ok1 || !ok2 {
		t.Fatal("missing flex item rects")
	}
	if r2.X <= r1.X {
		t.Fatalf("expected item2 to be placed to the right of item1: r1=%+v r2=%+v", r1, r2)
	}

	hitID, ok := HitTest(&rootBox, r1.X+5, r1.Y+5)
	if !ok {
		t.Fatal("HitTest missed item1")
	}
	_ = hitID

	selText := CollectTextRunsInRect(&rootBox, Rect{X: 0, Y: 0, Width: 800, Height: 600})
	if selText == "" {
		t.Fatal("expected non-empty text runs in viewport")
	}
}

func TestFlexOrderAspectRatioAndGridMixedTracks(t *testing.T) {
	html := `
	<div id="wrap" style="width: 400px;">
		<div id="flex" style="display: flex;">
			<div id="second" style="order: 2; width: 80px; height: 30px;">Second</div>
			<div id="first" style="order: 1; width: 80px; aspect-ratio: 2 / 1;">First</div>
		</div>
		<div id="grid" style="display: grid; grid-template-columns: 100px 1fr 2fr;">
			<div id="c1" style="height: 20px;">C1</div>
			<div id="c2" style="height: 20px;">C2</div>
			<div id="c3" style="height: 20px;">C3</div>
		</div>
	</div>`

	doc := dom.ParseHTML(html)
	sheet := style.ParseCSS("")
	sm := style.ComputeStylesWithViewport(doc, &sheet, 800, 600)
	rootBox := LayoutDocument(doc, sm, 800, 600)

	firstID, _ := doc.GetElementByID("first")
	secondID, _ := doc.GetElementByID("second")
	rFirst, _ := rootBox.FindNodeRect(firstID)
	rSecond, _ := rootBox.FindNodeRect(secondID)
	if rFirst.X >= rSecond.X {
		t.Fatalf("expected order:1 item before order:2 item: first=%+v second=%+v", rFirst, rSecond)
	}
	if rFirst.Height != 40 {
		t.Fatalf("expected aspect-ratio 2/1 on 80px width to yield height 40, got %v", rFirst.Height)
	}

	c1ID, _ := doc.GetElementByID("c1")
	c2ID, _ := doc.GetElementByID("c2")
	c3ID, _ := doc.GetElementByID("c3")
	rc1, _ := rootBox.FindNodeRect(c1ID)
	rc2, _ := rootBox.FindNodeRect(c2ID)
	rc3, _ := rootBox.FindNodeRect(c3ID)
	if rc1.Width != 100 || rc2.Width != 100 || rc3.Width != 200 {
		t.Fatalf("unexpected mixed grid column widths: c1=%v c2=%v c3=%v", rc1.Width, rc2.Width, rc3.Width)
	}
}

func TestIncrementalFlexAndGridSubtreeReuseMatchesFullLayout(t *testing.T) {
	var flexCards strings.Builder
	for i := 0; i < 24; i++ {
		fmt.Fprintf(&flexCards, `<div id="fc-%d" style="padding: 6px;"><h4>Card %d</h4><p id="fp-%d">Short %d</p></div>`, i, i, i, i)
	}
	flexHTML := `<div id="deck" style="display: flex; flex-direction: column; row-gap: 8px; width: 600px;">` + flexCards.String() + `</div>`
	flexDoc := dom.ParseHTML(flexHTML)
	sheet := style.ParseCSS("")
	sm0 := style.ComputeStylesWithViewport(flexDoc, &sheet, 800, 600)
	cache := NewLayoutCache()

	inc0 := LayoutDocumentIncremental(flexDoc, sm0, cache, 800, 600)
	full0 := LayoutDocument(flexDoc, sm0, 800, 600)
	if !reflect.DeepEqual(inc0, full0) {
		t.Fatal("initial incremental flex layout differed from full layout")
	}

	fp0, ok := flexDoc.GetElementByID("fp-0")
	if !ok {
		t.Fatal("missing fp-0")
	}
	flexDoc.SetTextContent(fp0, "Expanded multi-line paragraph content inside card zero that wraps across multiple lines and shifts cards 1..23 downward.")
	sm1 := style.ComputeStylesWithViewport(flexDoc, &sheet, 800, 600)
	beforeReuses := cache.SubtreeReuses
	inc1 := LayoutDocumentIncremental(flexDoc, sm1, cache, 800, 600)
	full1 := LayoutDocument(flexDoc, sm1, 800, 600)
	if got := cache.SubtreeReuses - beforeReuses; got < 23 {
		t.Fatalf("expected >= 23 flex sibling subtree reuses, got %d", got)
	}
	if !reflect.DeepEqual(inc1, full1) {
		t.Fatal("mutated incremental flex layout differed from full layout")
	}

	// Grid with fractional 1fr columns (`1000px` viewport -> fractional column widths)
	var gridCards strings.Builder
	for i := 0; i < 24; i++ {
		fmt.Fprintf(&gridCards, `<div id="gc-%d" style="padding: 8px;"><h4>Tile %d</h4><p id="gp-%d">Metric %d</p></div>`, i, i, i, i)
	}
	gridHTML := `<div id="board" style="display: grid; grid-template-columns: 1fr 1fr 1fr; column-gap: 12px; row-gap: 10px;">` + gridCards.String() + `</div>`
	gridDoc := dom.ParseHTML(gridHTML)
	gsm0 := style.ComputeStylesWithViewport(gridDoc, &sheet, 1000, 720)
	gcache := NewLayoutCache()
	ginc0 := LayoutDocumentIncremental(gridDoc, gsm0, gcache, 1000, 720)
	if !reflect.DeepEqual(ginc0, LayoutDocument(gridDoc, gsm0, 1000, 720)) {
		t.Fatal("initial incremental grid layout differed from full layout")
	}

	gp0, ok := gridDoc.GetElementByID("gp-0")
	if !ok {
		t.Fatal("missing gp-0")
	}
	gridDoc.SetTextContent(gp0, "Much longer metric description inside tile zero so row zero grows taller and rows 1..7 translate cleanly.")
	gsm1 := style.ComputeStylesWithViewport(gridDoc, &sheet, 1000, 720)
	gBefore := gcache.SubtreeReuses
	ginc1 := LayoutDocumentIncremental(gridDoc, gsm1, gcache, 1000, 720)
	if got := gcache.SubtreeReuses - gBefore; got < 23 {
		t.Fatalf("expected >= 23 grid sibling subtree reuses, got %d", got)
	}
	if !reflect.DeepEqual(ginc1, LayoutDocument(gridDoc, gsm1, 1000, 720)) {
		t.Fatal("mutated incremental grid layout differed from full layout")
	}

	// Viewport resize invalidates 1fr widths and matches full layout at 640x720
	gsmNarrow := style.ComputeStylesWithViewport(gridDoc, &sheet, 640, 720)
	gincNarrow := LayoutDocumentIncremental(gridDoc, gsmNarrow, gcache, 640, 720)
	if !reflect.DeepEqual(gincNarrow, LayoutDocument(gridDoc, gsmNarrow, 640, 720)) {
		t.Fatal("resized incremental grid layout differed from full layout")
	}
}
