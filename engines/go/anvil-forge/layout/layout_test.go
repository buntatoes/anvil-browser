// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato. Verify canonical repository history at https://github.com/buntatoes

package layout

import (
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
