// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato. Verify canonical repository history at https://github.com/buntatoes

package style

import (
	"testing"

	"github.com/buntatoes/anvil-forge/dom"
)

func TestCascadeLayersNestingAndCustomProps(t *testing.T) {
	html := `<div id="card" class="box"><span class="label">Hello</span></div>`
	css := `
	@layer base, theme;
	@layer theme {
		.box {
			--brand: #ff6b24;
			color: var(--brand);
			width: calc(200px + 40px);
			& .label {
				font-weight: bold;
				text-transform: uppercase;
			}
		}
	}
	@layer base {
		#card {
			color: #000000 !important;
			background: linear-gradient(90deg, #112233, #445566);
		}
	}`

	doc := dom.ParseHTML(html)
	sheet := ParseCSS(css)
	sm := ComputeStylesWithViewport(doc, &sheet, 1280, 720)

	cardID, ok := doc.GetElementByID("card")
	if !ok {
		t.Fatal("missing #card")
	}
	cardStyle, ok := sm.Get(cardID)
	if !ok {
		t.Fatal("missing computed style for #card")
	}
	// Earlier layer !important beats later layer normal
	if cardStyle.Color != RGBA(0, 0, 0, 255) {
		t.Fatalf("expected #000000 from @layer base !important, got %+v", cardStyle.Color)
	}
	if cardStyle.Width.Value != 240 {
		t.Fatalf("expected calc width 240px, got %+v", cardStyle.Width)
	}
	if cardStyle.BackgroundGradient == nil {
		t.Fatal("expected parsed linear-gradient on #card")
	}

	labels := doc.QuerySelectorAll(".label")
	if len(labels) != 1 {
		t.Fatalf("expected 1 .label, got %d", len(labels))
	}
	labelStyle, _ := sm.Get(labels[0])
	if labelStyle.FontWeight != 700 || labelStyle.TextTransform != TextTransformUppercase {
		t.Fatalf("nested selector rule did not apply: %+v", labelStyle)
	}
}

func TestIncrementalRelationalHasAndSiblingInvalidation(t *testing.T) {
	html := `<div id="card"><span id="trigger">A</span><span id="next">B</span></div>`
	css := `
	#card { opacity: 0.5; }
	#card:has(.on) { opacity: 1.0; }
	#next { font-weight: 400; }
	.on + #next { font-weight: 700; }
	`
	doc := dom.ParseHTML(html)
	sheet := ParseCSS(css)
	sm := ComputeStylesWithViewport(doc, &sheet, 800, 600)
	_ = doc.TakeCoalescedDirtySubtreeRoots()

	cardID, _ := doc.GetElementByID("card")
	triggerID, _ := doc.GetElementByID("trigger")
	nextID, _ := doc.GetElementByID("next")

	doc.SetAttribute(triggerID, "class", "on")
	sm, recomputed := ComputeStylesIncrementalCount(doc, &sheet, sm, 800, 600)
	if recomputed < 3 {
		t.Fatalf("expected relational/sibling promotion to recompute >= 3 nodes, got %d", recomputed)
	}
	if cs, _ := sm.Get(cardID); cs.Opacity != 1.0 {
		t.Fatalf("expected #card:has(.on) opacity 1.0, got %v", cs.Opacity)
	}
	if ns, _ := sm.Get(nextID); ns.FontWeight != 700 {
		t.Fatalf("expected .on + #next font-weight 700, got %d", ns.FontWeight)
	}
}

func TestPopoverLightDarkNestedMediaAndViewTransition(t *testing.T) {
	html := `<html id="root"><body>
		<div id="pop" popover>Popover</div>
		<div id="light_box" class="panel"><span id="child">Light</span></div>
		<div id="dark_box" class="panel"><span id="dark_child">Dark</span></div>
	</body></html>`
	css := `
	:root { color-scheme: light; }
	:root:active-view-transition #light_box { opacity: 0.4; }
	#pop:popover-open { width: 280px; }
	.panel {
		background-color: light-dark(#ffffff, #10141c);
		& span {
			color: light-dark(#224488, #8ab4f8);
		}
		@media (min-width: 600px) {
			padding: 18px;
		}
	}
	#dark_box {
		color-scheme: dark;
	}`
	doc := dom.ParseHTML(html)
	sheet := ParseCSS(css)
	sm := ComputeStylesWithViewport(doc, &sheet, 1024, 768)

	popID, _ := doc.GetElementByID("pop")
	lightBox, _ := doc.GetElementByID("light_box")
	childID, _ := doc.GetElementByID("child")
	darkBox, _ := doc.GetElementByID("dark_box")
	darkChild, _ := doc.GetElementByID("dark_child")

	if ps, _ := sm.Get(popID); ps.Display != DisplayNone {
		t.Fatalf("expected closed popover display:none, got %v", ps.Display)
	}
	if ls, _ := sm.Get(lightBox); ls.Background == nil || *ls.Background != RGBA(0xff, 0xff, 0xff, 255) || ls.Padding.Left != 18 {
		t.Fatalf("unexpected light_box style: %+v", ls)
	}
	if cs, _ := sm.Get(childID); cs.Color != RGBA(0x22, 0x44, 0x88, 255) {
		t.Fatalf("expected light child color #224488, got %+v", cs.Color)
	}
	if ds, _ := sm.Get(darkBox); ds.Background == nil || *ds.Background != RGBA(0x10, 0x14, 0x1c, 255) {
		t.Fatalf("expected dark_box background #10141c, got %+v", ds.Background)
	}
	if dcs, _ := sm.Get(darkChild); dcs.Color != RGBA(0x8a, 0xb4, 0xf8, 255) {
		t.Fatalf("expected dark_child inherited dark scheme color #8ab4f8, got %+v", dcs.Color)
	}

	if !doc.ShowPopover(popID) {
		t.Fatal("expected ShowPopover to return true")
	}
	doc.SetActiveViewTransition(true)
	sm2 := ComputeStylesWithViewport(doc, &sheet, 1024, 768)
	if ps2, _ := sm2.Get(popID); ps2.Display != DisplayBlock || ps2.Position != PositionFixed || ps2.Width.Value != 280 {
		t.Fatalf("expected open popover top-layer fixed 280px, got %+v", ps2)
	}
	if ls2, _ := sm2.Get(lightBox); ls2.Opacity != 0.4 {
		t.Fatalf("expected :active-view-transition opacity 0.4, got %v", ls2.Opacity)
	}
}
