// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato. Verify canonical repository history at https://github.com/buntatoes

package style

import (
	"math"
	"sort"
	"strconv"
	"strings"
	"sync"

	"github.com/buntatoes/anvil-forge/dom"
)

type ComputedStyle struct {
	Display            Display
	Position           Position
	Top                Size
	Right              Size
	Bottom             Size
	Left               Size
	Width              Size
	Height             Size
	MinWidth           Size
	MinHeight          Size
	MaxWidth           Size
	MaxHeight          Size
	Margin             Edge
	MarginAuto         [4]bool
	Padding            Edge
	Border             Edge
	BorderColor        Color
	BorderColors       [4]Color
	BorderRadius       float32
	OutlineWidth       float32
	OutlineColor       Color
	OutlineOffset      float32
	BoxSizing          BoxSizing
	Overflow           Overflow
	ScrollableOverflow bool
	ObjectFit          ObjectFit
	Visibility         Visibility
	ZIndex             int32
	Opacity            float32
	Color              Color
	Background         *Color
	BackgroundGradient *LinearGradient
	BoxShadow          *BoxShadow
	FontSize           float32
	LineHeight         float32
	FontWeight         uint16
	Italic             bool
	FontFamily         FontFamily
	TextAlign          TextAlign
	TextTransform      TextTransform
	WhiteSpace         WhiteSpace
	Underline          bool
	LineThrough        bool
	LetterSpacing      float32
	WordSpacing        float32
	TextOverflowEllip  bool
	BreakWord          bool
	AspectRatio        float32
	FlexDirection      FlexDirection
	FlexWrap           FlexWrap
	FlexGrow           float32
	FlexShrink         float32
	FlexBasis          Size
	Order              int32
	JustifyContent     JustifyContent
	AlignItems         AlignItems
	AlignSelf          *AlignItems
	AlignContent       AlignContent
	RowGap             float32
	ColumnGap          float32
	GridTemplateCols   []GridTrack
	GridTemplateRows   []GridTrack
	GridAutoFlow       GridAutoFlow
	Transforms         []TransformFn
	BeforeContent      string
	AfterContent       string
	Float              FloatSide
	Clear              ClearSide
	VerticalAlign      VerticalAlign
	BackgroundImage    string
	BorderRadii        [4]float32
	BorderStyles       [4]BorderStyle
	OverflowX          Overflow
	OverflowY          Overflow
	ContainerType      ContainerType
	ContainerNames     [4]string
	QueryContainerW    float32
	QueryContainerH    float32
	DarkColorScheme    bool
	CustomProps        map[string]string
}

func DefaultComputedStyle() ComputedStyle {
	black := RGBA(0, 0, 0, 255)
	return ComputedStyle{
		Display:      DisplayBlock,
		Position:     PositionStatic,
		Top:          Size{Kind: SizeAuto},
		Right:        Size{Kind: SizeAuto},
		Bottom:       Size{Kind: SizeAuto},
		Left:         Size{Kind: SizeAuto},
		Width:        Size{Kind: SizeAuto},
		Height:       Size{Kind: SizeAuto},
		MinWidth:     Size{Kind: SizeAuto},
		MinHeight:    Size{Kind: SizeAuto},
		MaxWidth:     Size{Kind: SizeAuto},
		MaxHeight:    Size{Kind: SizeAuto},
		BorderColor:  black,
		BorderColors: [4]Color{black, black, black, black},
		OutlineColor: black,
		BoxSizing:    BoxSizingContentBox,
		Overflow:     OverflowVisible,
		ObjectFit:    ObjectFitFill,
		Visibility:   VisibilityVisible,
		Opacity:      1.0,
		Color:        RGBA(17, 17, 17, 255),
		FontSize:     16.0,
		LineHeight:   20.0,
		FontWeight:   400,
		FontFamily:   FontSansSerif,
		TextAlign:    TextAlignLeft,
		WhiteSpace:   WhiteSpaceNormal,
		FlexShrink:   1.0,
		FlexBasis:    Size{Kind: SizeAuto},
		AlignItems:   AlignStretch,
		AlignContent: AlignContentStretch,
		CustomProps:  make(map[string]string),
	}
}

type StyleMap struct {
	Styles     map[dom.NodeId]ComputedStyle
	RootFontPx float32
	ViewportW  uint32
	ViewportH  uint32
}

func (sm *StyleMap) Get(id dom.NodeId) (ComputedStyle, bool) {
	st, ok := sm.Styles[id]
	return st, ok
}

type Animator struct {
	NowMs uint64
}

func NewAnimator() *Animator {
	return &Animator{}
}

func (a *Animator) Tick(nowMs uint64) bool {
	a.NowMs = nowMs
	return false
}

func ComputeStyles(doc *dom.Document, sheet *Stylesheet) *StyleMap {
	return ComputeStylesWithViewport(doc, sheet, 1280, 720)
}

func ComputeStylesWithViewport(doc *dom.Document, sheet *Stylesheet, vw, vh uint32) *StyleMap {
	return ComputeStylesWithContainers(doc, sheet, vw, vh, nil)
}

type containerFrame struct {
	ctype ContainerType
	names [4]string
	w     float32
	h     float32
}

func ComputeStylesWithContainers(doc *dom.Document, sheet *Stylesheet, vw, vh uint32, measured map[dom.NodeId][2]float32) *StyleMap {
	sm := &StyleMap{
		Styles:     make(map[dom.NodeId]ComputedStyle, doc.NodeCount()),
		RootFontPx: 16.0,
		ViewportW:  vw,
		ViewportH:  vh,
	}
	var walk func(dom.NodeId, *ComputedStyle, []containerFrame)
	walk = func(id dom.NodeId, parent *ComputedStyle, stack []containerFrame) {
		n := doc.Node(id)
		if n == nil {
			return
		}
		if n.Data.Kind == dom.NodeKindElement {
			st := resolveElementStyleWithContainers(doc, id, parent, sheet, sm.RootFontPx, float32(vw), float32(vh), stack)
			if id == doc.HTML() {
				sm.RootFontPx = st.FontSize
			}
			sm.Styles[id] = st
			childStack := stack
			if st.ContainerType != ContainerNormal {
				cw, ch := float32(vw), float32(vh)
				if len(stack) > 0 {
					cw, ch = stack[len(stack)-1].w, stack[len(stack)-1].h
				}
				if wPx, ok := st.Width.ResolvePxWithContainer(cw, st.FontSize, sm.RootFontPx, float32(vw), float32(vh), cw, ch); ok {
					cw = wPx
				}
				if st.ContainerType == ContainerSize {
					if hPx, ok := st.Height.ResolvePxWithContainer(ch, st.FontSize, sm.RootFontPx, float32(vw), float32(vh), cw, ch); ok {
						ch = hPx
					}
				}
				if measured != nil {
					if dims, ok := measured[id]; ok {
						cw = dims[0]
						if st.ContainerType == ContainerSize {
							ch = dims[1]
						}
					}
				}
				childStack = append(append([]containerFrame(nil), stack...), containerFrame{
					ctype: st.ContainerType,
					names: st.ContainerNames,
					w:     cw,
					h:     ch,
				})
			}
			for _, c := range doc.ComposedChildren(id) {
				walk(c, &st, childStack)
			}
		} else {
			for _, c := range n.Children {
				walk(c, parent, stack)
			}
		}
	}
	walk(doc.Root(), nil, nil)
	return sm
}

func RefineContainerStyles(doc *dom.Document, sheet *Stylesheet, sm *StyleMap, measured map[dom.NodeId][2]float32) bool {
	if doc == nil || sheet == nil || sm == nil || len(measured) == 0 {
		return false
	}
	next := ComputeStylesWithContainers(doc, sheet, sm.ViewportW, sm.ViewportH, measured)
	changed := false
	for id, nst := range next.Styles {
		if ost, ok := sm.Styles[id]; !ok || ost.FontSize != nst.FontSize || ost.Width != nst.Width ||
			ost.Height != nst.Height || ost.Display != nst.Display || ost.FlexDirection != nst.FlexDirection ||
			ost.Color != nst.Color || (ost.Background == nil) != (nst.Background == nil) ||
			(ost.Background != nil && nst.Background != nil && *ost.Background != *nst.Background) {
			changed = true
		}
		sm.Styles[id] = nst
	}
	sm.RootFontPx = next.RootFontPx
	return changed
}

func ComputeStylesIncremental(doc *dom.Document, sheet *Stylesheet, prev *StyleMap, vw, vh uint32) *StyleMap {
	sm, _ := ComputeStylesIncrementalCount(doc, sheet, prev, vw, vh)
	return sm
}

func ComputeStylesIncrementalCount(doc *dom.Document, sheet *Stylesheet, prev *StyleMap, vw, vh uint32) (*StyleMap, int) {
	roots := doc.TakeCoalescedDirtySubtreeRoots()
	if prev == nil || prev.Styles == nil || prev.ViewportW != vw || prev.ViewportH != vh {
		fresh := ComputeStylesWithViewport(doc, sheet, vw, vh)
		return fresh, len(fresh.Styles)
	}
	if len(roots) == 0 {
		return prev, 0
	}
	for _, r := range roots {
		if r == doc.Root() || r == doc.HTML() {
			fresh := ComputeStylesWithViewport(doc, sheet, vw, vh)
			return fresh, len(fresh.Styles)
		}
	}

	hasRelational := false
	hasSibling := false
	if sheet != nil {
		for _, rule := range sheet.Rules {
			for _, sel := range rule.Selectors {
				if strings.Contains(sel, ":has(") {
					hasRelational = true
				}
				if strings.Contains(sel, "+") || strings.Contains(sel, "~") || strings.Contains(sel, ":nth-") {
					hasSibling = true
				}
			}
		}
	}
	if hasRelational || hasSibling {
		promoted := make([]dom.NodeId, 0, len(roots))
		seen := make(map[dom.NodeId]bool, len(roots))
		for _, r := range roots {
			cur := r
			if hasRelational {
				for {
					pe, ok := doc.ParentElement(cur)
					if !ok || pe == doc.HTML() || pe == doc.Root() {
						break
					}
					cur = pe
				}
			} else if hasSibling {
				if pe, ok := doc.ParentElement(cur); ok && pe != doc.HTML() && pe != doc.Root() {
					cur = pe
				}
			}
			if !seen[cur] {
				seen[cur] = true
				promoted = append(promoted, cur)
			}
		}
		roots = promoted
	}

	recascaded := 0
	var walkSubtree func(dom.NodeId, *ComputedStyle)
	walkSubtree = func(id dom.NodeId, parent *ComputedStyle) {
		n := doc.Node(id)
		if n == nil {
			return
		}
		if n.Data.Kind == dom.NodeKindElement {
			st := resolveElementStyle(doc, id, parent, sheet, prev.RootFontPx, float32(vw), float32(vh))
			prev.Styles[id] = st
			recascaded++
			for _, c := range doc.ComposedChildren(id) {
				walkSubtree(c, &st)
			}
		} else {
			for _, c := range n.Children {
				walkSubtree(c, parent)
			}
		}
	}

	for _, subRoot := range roots {
		var parentStyle *ComputedStyle
		if pe, ok := doc.ParentElement(subRoot); ok {
			if pst, found := prev.Styles[pe]; found {
				cp := pst
				parentStyle = &cp
			}
		}
		walkSubtree(subRoot, parentStyle)
	}
	return prev, recascaded
}

func ComputeStylesParallel(doc *dom.Document, sheet *Stylesheet, vw, vh uint32) *StyleMap {
	var wg sync.WaitGroup
	var out *StyleMap
	wg.Add(1)
	go func() {
		defer wg.Done()
		out = ComputeStylesWithViewport(doc, sheet, vw, vh)
	}()
	wg.Wait()
	return out
}

func ComputeStylesAnimated(doc *dom.Document, sheet *Stylesheet, anim *Animator, vw, vh uint32) *StyleMap {
	if anim != nil {
		_ = anim.NowMs
	}
	return ComputeStylesWithViewport(doc, sheet, vw, vh)
}

type matchedDecl struct {
	decl        Declaration
	layerOrder  int
	scopeHops   int
	specificity Specificity
	sourceOrder int
	isInline    bool
}

func resolveElementStyle(doc *dom.Document, id dom.NodeId, parent *ComputedStyle, sheet *Stylesheet, rootFont, vw, vh float32) ComputedStyle {
	return resolveElementStyleWithContainers(doc, id, parent, sheet, rootFont, vw, vh, nil)
}

func resolveElementStyleWithContainers(doc *dom.Document, id dom.NodeId, parent *ComputedStyle, sheet *Stylesheet, rootFont, vw, vh float32, containers []containerFrame) ComputedStyle {
	st := DefaultComputedStyle()
	if parent != nil {
		st.Color = parent.Color
		st.FontSize = parent.FontSize
		st.LineHeight = parent.LineHeight
		st.FontWeight = parent.FontWeight
		st.Italic = parent.Italic
		st.FontFamily = parent.FontFamily
		st.TextAlign = parent.TextAlign
		st.TextTransform = parent.TextTransform
		st.WhiteSpace = parent.WhiteSpace
		st.Visibility = parent.Visibility
		st.LetterSpacing = parent.LetterSpacing
		st.WordSpacing = parent.WordSpacing
		st.DarkColorScheme = parent.DarkColorScheme
		for k, v := range parent.CustomProps {
			st.CustomProps[k] = v
		}
	}

	n := doc.Node(id)
	if n == nil {
		return st
	}
	applyUserAgentDefaults(&st, n)
	isClosedPopover := doc.IsPopover(id) && !doc.IsPopoverOpen(id)
	isOpenPopover := doc.IsPopoverOpen(id)
	if isClosedPopover {
		st.Display = DisplayNone
	} else if isOpenPopover {
		st.Display = DisplayBlock
		st.Position = PositionFixed
	}

	var matches []matchedDecl
	orderCounter := 0
	if sheet != nil {
		layerIdxMap := make(map[string]int, len(sheet.Layers))
		for idx, l := range sheet.Layers {
			layerIdxMap[l] = idx + 1
		}
		for _, rule := range sheet.Rules {
			if rule.Media != "" && !matchesMediaQuery(rule.Media, uint32(vw), uint32(vh)) {
				continue
			}
			if rule.Container != "" && !matchesContainerClauses(rule.Container, containers, uint32(vw), uint32(vh)) {
				continue
			}
			hops, inScope := checkScopeProximity(doc, id, rule.ScopeRoot, rule.ScopeLimit)
			if !inScope {
				continue
			}
			var bestSpec Specificity
			matched := false
			for _, sel := range rule.Selectors {
				// Handle ::before and ::after rules separately
				lowerSel := strings.ToLower(sel)
				if strings.HasSuffix(lowerSel, "::before") {
					baseSel := sel[:len(sel)-8]
					if baseSel == "" || doc.MatchesSelector(id, baseSel) {
						for _, d := range rule.Declarations {
							if d.Property == "content" {
								st.BeforeContent = evalContentValue(&n.Data, d.Value)
							}
						}
					}
					continue
				}
				if strings.HasSuffix(lowerSel, "::after") {
					baseSel := sel[:len(sel)-7]
					if baseSel == "" || doc.MatchesSelector(id, baseSel) {
						for _, d := range rule.Declarations {
							if d.Property == "content" {
								st.AfterContent = evalContentValue(&n.Data, d.Value)
							}
						}
					}
					continue
				}
				if doc.MatchesSelector(id, sel) {
					sp := ComputeSpecificity(sel)
					if !matched || bestSpec.Less(sp) {
						bestSpec = sp
						matched = true
					}
				}
			}
			if !matched {
				continue
			}
			lOrder := len(sheet.Layers) + 1 // unlayered beats layered for normal rules
			if rule.Layer != "" {
				if idx, ok := layerIdxMap[rule.Layer]; ok {
					lOrder = idx
				}
			}
			for _, d := range rule.Declarations {
				orderCounter++
				matches = append(matches, matchedDecl{
					decl:        d,
					layerOrder:  lOrder,
					scopeHops:   hops,
					specificity: bestSpec,
					sourceOrder: orderCounter,
				})
			}
		}
	}

	if inlineStyle, ok := n.Data.Attr("style"); ok && strings.TrimSpace(inlineStyle) != "" {
		for _, d := range parseDeclarationsOnly(inlineStyle) {
			orderCounter++
			matches = append(matches, matchedDecl{
				decl:        d,
				layerOrder:  999999,
				scopeHops:   0,
				specificity: Specificity{IDs: 1000},
				sourceOrder: orderCounter,
				isInline:    true,
			})
		}
	}

	sort.SliceStable(matches, func(i, j int) bool {
		a, b := matches[i], matches[j]
		if a.decl.Important != b.decl.Important {
			return !a.decl.Important && b.decl.Important
		}
		if a.isInline != b.isInline {
			return !a.isInline && b.isInline
		}
		if a.layerOrder != b.layerOrder {
			if a.decl.Important {
				return a.layerOrder > b.layerOrder
			}
			return a.layerOrder < b.layerOrder
		}
		if a.scopeHops != b.scopeHops {
			return a.scopeHops > b.scopeHops
		}
		if a.specificity != b.specificity {
			return a.specificity.Less(b.specificity)
		}
		return a.sourceOrder < b.sourceOrder
	})

	// Pass 1: custom properties (--*)
	for _, m := range matches {
		if strings.HasPrefix(m.decl.Property, "--") {
			st.CustomProps[m.decl.Property] = m.decl.Value
		}
	}
	// Pass 2: font-size first so em units resolve against updated font-size
	parentFont := float32(16.0)
	if parent != nil {
		parentFont = parent.FontSize
	}
	cqw, cqh := vw, vh
	if len(containers) > 0 {
		cqw, cqh = containers[len(containers)-1].w, containers[len(containers)-1].h
	}
	st.QueryContainerW = cqw
	st.QueryContainerH = cqh
	for _, m := range matches {
		if m.decl.Property == "color-scheme" {
			val := resolveCustomVars(m.decl.Value, st.CustomProps, 0)
			if dark, ok := parseColorSchemeKeyword(val); ok {
				st.DarkColorScheme = dark
			}
		} else if m.decl.Property == "font-size" {
			val := resolveCustomVars(m.decl.Value, st.CustomProps, 0)
			if sz := parseSizeValueWithContainer(val, parentFont, rootFont, vw, vh, cqw, cqh); sz.Kind != SizeAuto {
				if px, ok := sz.ResolvePxWithContainer(parentFont, parentFont, rootFont, vw, vh, cqw, cqh); ok && px > 0 {
					st.FontSize = px
					st.LineHeight = px * 1.25
				}
			}
		}
	}
	// Pass 3: all other declarations
	for _, m := range matches {
		if strings.HasPrefix(m.decl.Property, "--") || m.decl.Property == "font-size" {
			continue
		}
		val := resolveCustomVars(m.decl.Value, st.CustomProps, 0)
		val = resolveLightDarkCalls(val, st.DarkColorScheme)
		applyDeclarationWithContainer(&st, parent, m.decl.Property, val, st.FontSize, rootFont, vw, vh, cqw, cqh)
	}
	if isClosedPopover {
		st.Display = DisplayNone
	} else if isOpenPopover && st.ZIndex == 0 {
		st.ZIndex = 2147483000
	}
	return st
}

func parseColorSchemeKeyword(raw string) (bool, bool) {
	for _, tok := range strings.Fields(strings.ToLower(raw)) {
		if tok == "only" {
			continue
		}
		if tok == "dark" {
			return true, true
		}
		if tok == "light" || tok == "normal" {
			return false, true
		}
	}
	return false, false
}

func resolveLightDarkCalls(input string, darkScheme bool) string {
	lower := strings.ToLower(input)
	if !strings.Contains(lower, "light-dark(") {
		return input
	}
	cur := input
	for step := 0; step < 8; step++ {
		idx := strings.Index(strings.ToLower(cur), "light-dark(")
		if idx < 0 {
			break
		}
		openParen := idx + len("light-dark")
		closeParen := findMatchingParenStr(cur, openParen)
		if closeParen <= openParen {
			break
		}
		inner := cur[openParen+1 : closeParen]
		parts := splitSelectors(inner)
		if len(parts) != 2 {
			break
		}
		chosen := strings.TrimSpace(parts[0])
		if darkScheme {
			chosen = strings.TrimSpace(parts[1])
		}
		cur = cur[:idx] + chosen + cur[closeParen+1:]
	}
	return cur
}

func applyUserAgentDefaults(st *ComputedStyle, n *dom.Node) {
	switch n.Data.Tag {
	case "head", "style", "script", "title", "meta", "link", "template", "noscript":
		st.Display = DisplayNone
	case "span", "a", "b", "strong", "i", "em", "u", "s", "code", "small", "label":
		st.Display = DisplayInline
		if n.Data.Tag == "a" {
			st.Color = RGBA(0, 102, 204, 255)
			st.Underline = true
		} else if n.Data.Tag == "b" || n.Data.Tag == "strong" {
			st.FontWeight = 700
		} else if n.Data.Tag == "i" || n.Data.Tag == "em" {
			st.Italic = true
		} else if n.Data.Tag == "u" {
			st.Underline = true
		} else if n.Data.Tag == "s" {
			st.LineThrough = true
		} else if n.Data.Tag == "code" {
			st.FontFamily = FontMonospace
		}
	case "button", "input", "select", "textarea", "img", "video", "canvas", "progress", "meter":
		st.Display = DisplayInlineBlock
	case "h1":
		st.FontSize = 32
		st.LineHeight = 40
		st.FontWeight = 700
	case "h2":
		st.FontSize = 24
		st.LineHeight = 30
		st.FontWeight = 700
	case "h3":
		st.FontSize = 19
		st.LineHeight = 24
		st.FontWeight = 700
	case "pre":
		st.FontFamily = FontMonospace
		st.WhiteSpace = WhiteSpacePre
	case "ul", "ol":
		st.Padding.Left = 24
	case "li":
		st.Display = DisplayListItem
	case "table":
		st.Display = DisplayTable
	case "thead":
		st.Display = DisplayTableHeaderGroup
	case "tbody":
		st.Display = DisplayTableRowGroup
	case "tfoot":
		st.Display = DisplayTableFooterGroup
	case "tr":
		st.Display = DisplayTableRow
	case "td", "th":
		st.Display = DisplayTableCell
		if n.Data.Tag == "th" {
			st.FontWeight = 700
		}
	case "dialog":
		if _, open := n.Data.Attr("open"); !open {
			st.Display = DisplayNone
		}
	}
	if _, hidden := n.Data.Attr("hidden"); hidden {
		st.Display = DisplayNone
	}
}

func applyDeclaration(st *ComputedStyle, parent *ComputedStyle, prop, rawVal string, fontSize, rootFont, vw, vh float32) {
	applyDeclarationWithContainer(st, parent, prop, rawVal, fontSize, rootFont, vw, vh, vw, vh)
}

func applyDeclarationWithContainer(st *ComputedStyle, parent *ComputedStyle, prop, rawVal string, fontSize, rootFont, vw, vh, cqw, cqh float32) {
	val := strings.TrimSpace(rawVal)
	lower := strings.ToLower(val)
	if lower == "initial" || lower == "unset" || lower == "revert" || lower == "revert-layer" {
		def := DefaultComputedStyle()
		switch prop {
		case "color":
			if lower == "unset" && parent != nil {
				st.Color = parent.Color
			} else {
				st.Color = def.Color
			}
		case "background", "background-color":
			st.Background = nil
			st.BackgroundGradient = nil
		case "display":
			st.Display = def.Display
		}
		return
	}
	if lower == "inherit" && parent != nil {
		switch prop {
		case "color":
			st.Color = parent.Color
		case "background-color":
			st.Background = parent.Background
		case "font-size":
			st.FontSize = parent.FontSize
		}
		return
	}

	switch prop {
	case "display":
		switch lower {
		case "none":
			st.Display = DisplayNone
		case "block":
			st.Display = DisplayBlock
		case "inline":
			st.Display = DisplayInline
		case "inline-block":
			st.Display = DisplayInlineBlock
		case "list-item":
			st.Display = DisplayListItem
		case "flow-root":
			st.Display = DisplayFlowRoot
		case "flex":
			st.Display = DisplayFlex
		case "inline-flex":
			st.Display = DisplayInlineFlex
		case "grid":
			st.Display = DisplayGrid
		case "inline-grid":
			st.Display = DisplayInlineGrid
		case "table":
			st.Display = DisplayTable
		case "table-row":
			st.Display = DisplayTableRow
		case "table-cell":
			st.Display = DisplayTableCell
		case "contents":
			st.Display = DisplayContents
		}
	case "position":
		switch lower {
		case "static":
			st.Position = PositionStatic
		case "relative":
			st.Position = PositionRelative
		case "absolute":
			st.Position = PositionAbsolute
		case "fixed":
			st.Position = PositionFixed
		case "sticky":
			st.Position = PositionSticky
		}
	case "top":
		st.Top = parseSizeValueWithContainer(val, fontSize, rootFont, vw, vh, cqw, cqh)
	case "right":
		st.Right = parseSizeValueWithContainer(val, fontSize, rootFont, vw, vh, cqw, cqh)
	case "bottom":
		st.Bottom = parseSizeValueWithContainer(val, fontSize, rootFont, vw, vh, cqw, cqh)
	case "left":
		st.Left = parseSizeValueWithContainer(val, fontSize, rootFont, vw, vh, cqw, cqh)
	case "inset":
		sz := parseSizeValueWithContainer(val, fontSize, rootFont, vw, vh, cqw, cqh)
		st.Top, st.Right, st.Bottom, st.Left = sz, sz, sz, sz
	case "width", "inline-size":
		st.Width = parseSizeValueWithContainer(val, fontSize, rootFont, vw, vh, cqw, cqh)
	case "height", "block-size":
		st.Height = parseSizeValueWithContainer(val, fontSize, rootFont, vw, vh, cqw, cqh)
	case "min-width", "min-inline-size":
		st.MinWidth = parseSizeValueWithContainer(val, fontSize, rootFont, vw, vh, cqw, cqh)
	case "min-height", "min-block-size":
		st.MinHeight = parseSizeValueWithContainer(val, fontSize, rootFont, vw, vh, cqw, cqh)
	case "max-width", "max-inline-size":
		st.MaxWidth = parseSizeValueWithContainer(val, fontSize, rootFont, vw, vh, cqw, cqh)
	case "max-height", "max-block-size":
		st.MaxHeight = parseSizeValueWithContainer(val, fontSize, rootFont, vw, vh, cqw, cqh)
	case "box-sizing":
		if lower == "border-box" {
			st.BoxSizing = BoxSizingBorderBox
		} else if lower == "content-box" {
			st.BoxSizing = BoxSizingContentBox
		}
	case "float":
		switch lower {
		case "left", "inline-start":
			st.Float = FloatLeft
		case "right", "inline-end":
			st.Float = FloatRight
		default:
			st.Float = FloatNone
		}
	case "clear":
		switch lower {
		case "left", "inline-start":
			st.Clear = ClearLeft
		case "right", "inline-end":
			st.Clear = ClearRight
		case "both":
			st.Clear = ClearBoth
		default:
			st.Clear = ClearNone
		}
	case "vertical-align":
		switch lower {
		case "middle", "center":
			st.VerticalAlign = VerticalAlignMiddle
		case "top", "text-top":
			st.VerticalAlign = VerticalAlignTop
		case "bottom", "text-bottom":
			st.VerticalAlign = VerticalAlignBottom
		case "sub":
			st.VerticalAlign = VerticalAlignSub
		case "super":
			st.VerticalAlign = VerticalAlignSuper
		default:
			st.VerticalAlign = VerticalAlignBaseline
		}
	case "margin":
		e, autos := parseBoxEdgeShorthand(val, fontSize, rootFont, vw, vh)
		st.Margin = e
		st.MarginAuto = autos
	case "margin-top", "margin-block-start":
		if lower == "auto" {
			st.MarginAuto[0] = true
		} else {
			st.Margin.Top = EvalMathPx(val, fontSize, rootFont, vw, vh)
		}
	case "margin-right", "margin-inline-end":
		if lower == "auto" {
			st.MarginAuto[1] = true
		} else {
			st.Margin.Right = EvalMathPx(val, fontSize, rootFont, vw, vh)
		}
	case "margin-bottom", "margin-block-end":
		if lower == "auto" {
			st.MarginAuto[2] = true
		} else {
			st.Margin.Bottom = EvalMathPx(val, fontSize, rootFont, vw, vh)
		}
	case "margin-left", "margin-inline-start":
		if lower == "auto" {
			st.MarginAuto[3] = true
		} else {
			st.Margin.Left = EvalMathPx(val, fontSize, rootFont, vw, vh)
		}
	case "padding":
		e, _ := parseBoxEdgeShorthand(val, fontSize, rootFont, vw, vh)
		st.Padding = e
	case "padding-top", "padding-block-start":
		st.Padding.Top = EvalMathPx(val, fontSize, rootFont, vw, vh)
	case "padding-right", "padding-inline-end":
		st.Padding.Right = EvalMathPx(val, fontSize, rootFont, vw, vh)
	case "padding-bottom", "padding-block-end":
		st.Padding.Bottom = EvalMathPx(val, fontSize, rootFont, vw, vh)
	case "padding-left", "padding-inline-start":
		st.Padding.Left = EvalMathPx(val, fontSize, rootFont, vw, vh)
	case "border", "border-width":
		w, col, hasCol, bs := parseBorderShorthandWithStyle(val, fontSize, rootFont, vw, vh)
		st.Border = Edge{Top: w, Right: w, Bottom: w, Left: w}
		st.BorderStyles = [4]BorderStyle{bs, bs, bs, bs}
		if hasCol {
			st.BorderColor = col
			st.BorderColors = [4]Color{col, col, col, col}
		}
	case "border-style":
		bs := parseBorderStyleKeyword(lower)
		st.BorderStyles = [4]BorderStyle{bs, bs, bs, bs}
		if bs == BorderStyleNone || bs == BorderStyleHidden {
			st.Border = Edge{}
		}
	case "border-top-style":
		st.BorderStyles[0] = parseBorderStyleKeyword(lower)
		if st.BorderStyles[0] == BorderStyleNone || st.BorderStyles[0] == BorderStyleHidden {
			st.Border.Top = 0
		}
	case "border-right-style":
		st.BorderStyles[1] = parseBorderStyleKeyword(lower)
		if st.BorderStyles[1] == BorderStyleNone || st.BorderStyles[1] == BorderStyleHidden {
			st.Border.Right = 0
		}
	case "border-bottom-style":
		st.BorderStyles[2] = parseBorderStyleKeyword(lower)
		if st.BorderStyles[2] == BorderStyleNone || st.BorderStyles[2] == BorderStyleHidden {
			st.Border.Bottom = 0
		}
	case "border-left-style":
		st.BorderStyles[3] = parseBorderStyleKeyword(lower)
		if st.BorderStyles[3] == BorderStyleNone || st.BorderStyles[3] == BorderStyleHidden {
			st.Border.Left = 0
		}
	case "border-color":
		if c, ok := ParseColor(val); ok {
			st.BorderColor = c
			st.BorderColors = [4]Color{c, c, c, c}
		}
	case "border-top-color":
		if c, ok := ParseColor(val); ok {
			st.BorderColors[0] = c
		}
	case "border-right-color":
		if c, ok := ParseColor(val); ok {
			st.BorderColors[1] = c
		}
	case "border-bottom-color":
		if c, ok := ParseColor(val); ok {
			st.BorderColors[2] = c
		}
	case "border-left-color":
		if c, ok := ParseColor(val); ok {
			st.BorderColors[3] = c
		}
	case "border-radius":
		e, _ := parseBoxEdgeShorthand(val, fontSize, rootFont, vw, vh)
		st.BorderRadii = [4]float32{e.Top, e.Right, e.Bottom, e.Left}
		st.BorderRadius = e.Top
	case "border-top-left-radius", "border-start-start-radius":
		r := EvalMathPx(val, fontSize, rootFont, vw, vh)
		st.BorderRadii[0] = r
		st.BorderRadius = r
	case "border-top-right-radius", "border-start-end-radius":
		st.BorderRadii[1] = EvalMathPx(val, fontSize, rootFont, vw, vh)
	case "border-bottom-right-radius", "border-end-end-radius":
		st.BorderRadii[2] = EvalMathPx(val, fontSize, rootFont, vw, vh)
	case "border-bottom-left-radius", "border-end-start-radius":
		st.BorderRadii[3] = EvalMathPx(val, fontSize, rootFont, vw, vh)
	case "outline":
		w, col, hasCol := parseBorderShorthand(val, fontSize, rootFont, vw, vh)
		st.OutlineWidth = w
		if hasCol {
			st.OutlineColor = col
		}
	case "outline-offset":
		st.OutlineOffset = EvalMathPx(val, fontSize, rootFont, vw, vh)
	case "color":
		if c, ok := ParseColor(val); ok {
			st.Color = c
		}
	case "background", "background-color", "background-image":
		if strings.Contains(lower, "url(") {
			st.BackgroundImage = extractURL(val)
		}
		if strings.Contains(lower, "gradient(") {
			if lg, ok := parseLinearGradient(val); ok {
				st.BackgroundGradient = &lg
			}
		} else if c, ok := ParseColor(val); ok {
			st.Background = &c
		}
	case "box-shadow":
		if bs, ok := parseBoxShadow(val, fontSize, rootFont, vw, vh); ok {
			st.BoxShadow = &bs
		}
	case "opacity":
		if v, err := strconv.ParseFloat(strings.TrimSuffix(lower, "%"), 32); err == nil {
			if strings.HasSuffix(lower, "%") {
				v /= 100.0
			}
			if v < 0 {
				v = 0
			} else if v > 1 {
				v = 1
			}
			st.Opacity = float32(v)
		}
	case "z-index":
		if v, err := strconv.Atoi(lower); err == nil {
			st.ZIndex = int32(v)
		}
	case "overflow":
		toks := strings.Fields(lower)
		if len(toks) == 1 {
			ov, sc := parseOverflowMode(toks[0])
			st.Overflow, st.OverflowX, st.OverflowY = ov, ov, ov
			st.ScrollableOverflow = sc
		} else if len(toks) >= 2 {
			ox, sx := parseOverflowMode(toks[0])
			oy, sy := parseOverflowMode(toks[1])
			st.OverflowX, st.OverflowY = ox, oy
			if sx || sy {
				st.Overflow = OverflowAuto
				st.ScrollableOverflow = true
			} else if ox != OverflowVisible || oy != OverflowVisible {
				st.Overflow = OverflowHidden
			} else {
				st.Overflow = OverflowVisible
			}
		}
	case "overflow-x", "overflow-inline":
		ox, sx := parseOverflowMode(lower)
		st.OverflowX = ox
		if sx {
			st.Overflow = OverflowAuto
			st.ScrollableOverflow = true
		} else if ox != OverflowVisible && st.Overflow == OverflowVisible {
			st.Overflow = OverflowHidden
		}
	case "overflow-y", "overflow-block":
		oy, sy := parseOverflowMode(lower)
		st.OverflowY = oy
		if sy {
			st.Overflow = OverflowAuto
			st.ScrollableOverflow = true
		} else if oy != OverflowVisible && st.Overflow == OverflowVisible {
			st.Overflow = OverflowHidden
		}
	case "container-type":
		st.ContainerType = parseContainerTypeKeyword(lower)
	case "container-name":
		st.ContainerNames = parseContainerNamesList(val)
	case "container":
		if parts := strings.SplitN(val, "/", 2); len(parts) == 2 {
			st.ContainerNames = parseContainerNamesList(parts[0])
			st.ContainerType = parseContainerTypeKeyword(strings.ToLower(strings.TrimSpace(parts[1])))
		} else if ct := parseContainerTypeKeyword(lower); ct != ContainerNormal || lower == "normal" {
			st.ContainerType = ct
		} else {
			st.ContainerNames = parseContainerNamesList(val)
		}
	case "visibility":
		if lower == "hidden" || lower == "collapse" {
			st.Visibility = VisibilityHidden
		} else {
			st.Visibility = VisibilityVisible
		}
	case "object-fit":
		switch lower {
		case "contain":
			st.ObjectFit = ObjectFitContain
		case "cover":
			st.ObjectFit = ObjectFitCover
		case "none":
			st.ObjectFit = ObjectFitNone
		case "scale-down":
			st.ObjectFit = ObjectFitScaleDown
		default:
			st.ObjectFit = ObjectFitFill
		}
	case "line-height":
		if v, err := strconv.ParseFloat(lower, 32); err == nil {
			st.LineHeight = st.FontSize * float32(v)
		} else {
			st.LineHeight = EvalMathPx(val, fontSize, rootFont, vw, vh)
		}
	case "font-weight":
		if lower == "bold" || lower == "bolder" {
			st.FontWeight = 700
		} else if lower == "normal" || lower == "lighter" {
			st.FontWeight = 400
		} else if v, err := strconv.Atoi(lower); err == nil {
			st.FontWeight = uint16(v)
		}
	case "font-style":
		st.Italic = strings.Contains(lower, "italic") || strings.Contains(lower, "oblique")
	case "font-family":
		if strings.Contains(lower, "mono") || strings.Contains(lower, "consolas") || strings.Contains(lower, "courier") {
			st.FontFamily = FontMonospace
		} else if strings.Contains(lower, "serif") && !strings.Contains(lower, "sans-serif") {
			st.FontFamily = FontSerif
		} else {
			st.FontFamily = FontSansSerif
		}
	case "text-align":
		switch lower {
		case "center":
			st.TextAlign = TextAlignCenter
		case "right", "end":
			st.TextAlign = TextAlignRight
		case "justify":
			st.TextAlign = TextAlignJustify
		default:
			st.TextAlign = TextAlignLeft
		}
	case "text-transform":
		switch lower {
		case "uppercase":
			st.TextTransform = TextTransformUppercase
		case "lowercase":
			st.TextTransform = TextTransformLowercase
		case "capitalize":
			st.TextTransform = TextTransformCapitalize
		default:
			st.TextTransform = TextTransformNone
		}
	case "text-decoration", "text-decoration-line":
		st.Underline = strings.Contains(lower, "underline")
		st.LineThrough = strings.Contains(lower, "line-through")
	case "letter-spacing":
		st.LetterSpacing = EvalMathPx(val, fontSize, rootFont, vw, vh)
	case "word-spacing":
		st.WordSpacing = EvalMathPx(val, fontSize, rootFont, vw, vh)
	case "white-space":
		switch lower {
		case "pre":
			st.WhiteSpace = WhiteSpacePre
		case "nowrap":
			st.WhiteSpace = WhiteSpaceNoWrap
		case "pre-wrap":
			st.WhiteSpace = WhiteSpacePreWrap
		default:
			st.WhiteSpace = WhiteSpaceNormal
		}
	case "text-overflow":
		st.TextOverflowEllip = lower == "ellipsis"
	case "overflow-wrap", "word-break":
		st.BreakWord = strings.Contains(lower, "break")
	case "flex-direction":
		switch lower {
		case "row-reverse":
			st.FlexDirection = FlexRowReverse
		case "column":
			st.FlexDirection = FlexColumn
		case "column-reverse":
			st.FlexDirection = FlexColumnReverse
		default:
			st.FlexDirection = FlexRow
		}
	case "flex-wrap":
		switch lower {
		case "wrap":
			st.FlexWrap = FlexWrapNormal
		case "wrap-reverse":
			st.FlexWrap = FlexWrapReverse
		default:
			st.FlexWrap = FlexNoWrap
		}
	case "flex-grow":
		if v, err := strconv.ParseFloat(lower, 32); err == nil {
			st.FlexGrow = float32(v)
		}
	case "flex-shrink":
		if v, err := strconv.ParseFloat(lower, 32); err == nil {
			st.FlexShrink = float32(v)
		}
	case "flex-basis":
		st.FlexBasis = parseSizeValue(val, fontSize, rootFont, vw, vh)
	case "flex":
		fields := strings.Fields(lower)
		if len(fields) >= 1 {
			if fields[0] == "none" {
				st.FlexGrow = 0
				st.FlexShrink = 0
			} else if v, err := strconv.ParseFloat(fields[0], 32); err == nil {
				st.FlexGrow = float32(v)
				st.FlexShrink = 1
				st.FlexBasis = Size{Kind: SizePx, Value: 0}
			}
		}
		if len(fields) >= 2 {
			if v, err := strconv.ParseFloat(fields[1], 32); err == nil {
				st.FlexShrink = float32(v)
			}
		}
		if len(fields) >= 3 {
			st.FlexBasis = parseSizeValue(fields[2], fontSize, rootFont, vw, vh)
		}
	case "justify-content":
		switch lower {
		case "center":
			st.JustifyContent = JustifyCenter
		case "flex-end", "end", "right":
			st.JustifyContent = JustifyFlexEnd
		case "space-between":
			st.JustifyContent = JustifySpaceBetween
		case "space-around":
			st.JustifyContent = JustifySpaceAround
		case "space-evenly":
			st.JustifyContent = JustifySpaceEvenly
		default:
			st.JustifyContent = JustifyFlexStart
		}
	case "align-items":
		st.AlignItems = parseAlignItems(lower)
	case "align-self":
		ai := parseAlignItems(lower)
		st.AlignSelf = &ai
	case "gap", "grid-gap":
		fields := strings.Fields(val)
		if len(fields) == 1 {
			g := EvalMathPx(fields[0], fontSize, rootFont, vw, vh)
			st.RowGap = g
			st.ColumnGap = g
		} else if len(fields) >= 2 {
			st.RowGap = EvalMathPx(fields[0], fontSize, rootFont, vw, vh)
			st.ColumnGap = EvalMathPx(fields[1], fontSize, rootFont, vw, vh)
		}
	case "row-gap", "grid-row-gap":
		st.RowGap = EvalMathPx(val, fontSize, rootFont, vw, vh)
	case "column-gap", "grid-column-gap":
		st.ColumnGap = EvalMathPx(val, fontSize, rootFont, vw, vh)
	case "grid-template-columns":
		st.GridTemplateCols = parseGridTracks(val, fontSize, rootFont, vw, vh)
	case "grid-template-rows":
		st.GridTemplateRows = parseGridTracks(val, fontSize, rootFont, vw, vh)
	case "order":
		if v, err := strconv.Atoi(strings.TrimSpace(lower)); err == nil {
			st.Order = int32(v)
		}
	case "aspect-ratio":
		if parts := strings.Split(lower, "/"); len(parts) == 2 {
			w, err1 := strconv.ParseFloat(strings.TrimSpace(parts[0]), 32)
			h, err2 := strconv.ParseFloat(strings.TrimSpace(parts[1]), 32)
			if err1 == nil && err2 == nil && w > 0 && h > 0 {
				st.AspectRatio = float32(w / h)
			}
		} else if v, err := strconv.ParseFloat(strings.TrimSpace(lower), 32); err == nil && v > 0 {
			st.AspectRatio = float32(v)
		}
	}
}

func parseAlignItems(lower string) AlignItems {
	switch lower {
	case "flex-start", "start":
		return AlignFlexStart
	case "center":
		return AlignCenter
	case "flex-end", "end":
		return AlignFlexEnd
	case "baseline":
		return AlignBaseline
	default:
		return AlignStretch
	}
}

func parseGridTracks(raw string, fontSize, rootFont, vw, vh float32) []GridTrack {
	var out []GridTrack
	for _, tok := range strings.Fields(strings.ToLower(raw)) {
		if strings.HasSuffix(tok, "fr") {
			v, _ := strconv.ParseFloat(strings.TrimSuffix(tok, "fr"), 32)
			if v <= 0 {
				v = 1
			}
			out = append(out, GridTrack{Kind: SizeAuto, Fr: float32(v)})
		} else {
			sz := parseSizeValue(tok, fontSize, rootFont, vw, vh)
			if sz.Kind == SizeAuto {
				out = append(out, GridTrack{Kind: SizeAuto, Fr: 1})
			} else {
				out = append(out, GridTrack{Kind: sz.Kind, Value: sz.Value})
			}
		}
	}
	return out
}

func parseBoxEdgeShorthand(raw string, fontSize, rootFont, vw, vh float32) (Edge, [4]bool) {
	fields := strings.Fields(raw)
	var vals [4]float32
	var autos [4]bool
	parseOne := func(tok string) (float32, bool) {
		if strings.EqualFold(tok, "auto") {
			return 0, true
		}
		return EvalMathPx(tok, fontSize, rootFont, vw, vh), false
	}
	switch len(fields) {
	case 1:
		v, a := parseOne(fields[0])
		vals = [4]float32{v, v, v, v}
		autos = [4]bool{a, a, a, a}
	case 2:
		v0, a0 := parseOne(fields[0])
		v1, a1 := parseOne(fields[1])
		vals = [4]float32{v0, v1, v0, v1}
		autos = [4]bool{a0, a1, a0, a1}
	case 3:
		v0, a0 := parseOne(fields[0])
		v1, a1 := parseOne(fields[1])
		v2, a2 := parseOne(fields[2])
		vals = [4]float32{v0, v1, v2, v1}
		autos = [4]bool{a0, a1, a2, a1}
	default:
		if len(fields) >= 4 {
			for k := 0; k < 4; k++ {
				vals[k], autos[k] = parseOne(fields[k])
			}
		}
	}
	return Edge{Top: vals[0], Right: vals[1], Bottom: vals[2], Left: vals[3]}, autos
}

func parseBorderStyleKeyword(s string) BorderStyle {
	fields := strings.Fields(strings.ToLower(strings.TrimSpace(s)))
	if len(fields) == 0 {
		return BorderStyleSolid
	}
	switch fields[0] {
	case "none":
		return BorderStyleNone
	case "hidden":
		return BorderStyleHidden
	case "dashed":
		return BorderStyleDashed
	case "dotted":
		return BorderStyleDotted
	case "double":
		return BorderStyleDouble
	default:
		return BorderStyleSolid
	}
}

func parseOverflowMode(s string) (Overflow, bool) {
	switch strings.TrimSpace(s) {
	case "hidden", "clip":
		return OverflowHidden, false
	case "auto":
		return OverflowAuto, true
	case "scroll":
		return OverflowScroll, true
	default:
		return OverflowVisible, false
	}
}

func parseContainerTypeKeyword(s string) ContainerType {
	switch strings.TrimSpace(s) {
	case "inline-size":
		return ContainerInlineSize
	case "size":
		return ContainerSize
	default:
		return ContainerNormal
	}
}

func parseContainerNamesList(raw string) [4]string {
	var out [4]string
	if strings.EqualFold(strings.TrimSpace(raw), "none") {
		return out
	}
	idx := 0
	for _, tok := range strings.Fields(strings.ToLower(raw)) {
		if idx >= 4 {
			break
		}
		out[idx] = tok
		idx++
	}
	return out
}

func parseBorderShorthandWithStyle(raw string, fontSize, rootFont, vw, vh float32) (float32, Color, bool, BorderStyle) {
	w := float32(1.0)
	var col Color
	hasCol := false
	bs := BorderStyleSolid
	for _, tok := range strings.Fields(raw) {
		lower := strings.ToLower(tok)
		switch lower {
		case "solid":
			bs = BorderStyleSolid
			continue
		case "dashed":
			bs = BorderStyleDashed
			continue
		case "dotted":
			bs = BorderStyleDotted
			continue
		case "double":
			bs = BorderStyleDouble
			continue
		case "none":
			bs = BorderStyleNone
			w = 0
			continue
		case "hidden":
			bs = BorderStyleHidden
			w = 0
			continue
		}
		if c, ok := ParseColor(tok); ok {
			col = c
			hasCol = true
			continue
		}
		w = EvalMathPx(tok, fontSize, rootFont, vw, vh)
	}
	return w, col, hasCol, bs
}

func parseBorderShorthand(raw string, fontSize, rootFont, vw, vh float32) (float32, Color, bool) {
	w, col, hasCol, _ := parseBorderShorthandWithStyle(raw, fontSize, rootFont, vw, vh)
	return w, col, hasCol
}

func parseLinearGradient(raw string) (LinearGradient, bool) {
	lowerRaw := strings.ToLower(raw)
	kind := GradLinear
	prefix := "linear-gradient("
	switch {
	case strings.Contains(lowerRaw, "repeating-linear-gradient("):
		kind = GradRepeatingLinear
		prefix = "repeating-linear-gradient("
	case strings.Contains(lowerRaw, "repeating-radial-gradient("):
		kind = GradRepeatingRadial
		prefix = "repeating-radial-gradient("
	case strings.Contains(lowerRaw, "radial-gradient("):
		kind = GradRadial
		prefix = "radial-gradient("
	case strings.Contains(lowerRaw, "conic-gradient("):
		kind = GradConic
		prefix = "conic-gradient("
	}
	idx := strings.Index(lowerRaw, prefix)
	if idx < 0 {
		return LinearGradient{}, false
	}
	sub := raw[idx+len(prefix):]
	end := strings.LastIndexByte(sub, ')')
	if end >= 0 {
		sub = sub[:end]
	}
	parts := splitSelectors(sub)
	if len(parts) < 2 {
		return LinearGradient{}, false
	}
	angle := float32(180.0)
	if kind == GradConic {
		angle = 0
	}
	startIdx := 0
	p0 := strings.ToLower(strings.TrimSpace(parts[0]))
	if strings.HasPrefix(p0, "from ") {
		angStr := strings.TrimSpace(strings.Split(p0[5:], "at")[0])
		if strings.HasSuffix(angStr, "deg") {
			if v, err := strconv.ParseFloat(strings.TrimSuffix(angStr, "deg"), 32); err == nil {
				angle = float32(v)
			}
		}
		startIdx = 1
	} else if kind == GradRadial || kind == GradRepeatingRadial {
		if strings.HasPrefix(p0, "circle") || strings.HasPrefix(p0, "ellipse") || strings.Contains(p0, "at ") {
			startIdx = 1
		}
	} else if strings.HasSuffix(p0, "deg") {
		if v, err := strconv.ParseFloat(strings.TrimSuffix(p0, "deg"), 32); err == nil {
			angle = float32(v)
			startIdx = 1
		}
	} else if strings.HasPrefix(p0, "to ") {
		switch strings.TrimSpace(p0[3:]) {
		case "right":
			angle = 90
		case "left":
			angle = 270
		case "top":
			angle = 0
		case "bottom":
			angle = 180
		}
		startIdx = 1
	}
	stopParts := parts[startIdx:]
	if len(stopParts) < 2 {
		return LinearGradient{}, false
	}
	type parsedStop struct {
		col    Color
		pos    float32
		hasPos bool
	}
	var parsed []parsedStop
	for _, sp := range stopParts {
		sp = strings.TrimSpace(sp)
		if sp == "" {
			continue
		}
		// Extract optional trailing percentage stop position
		colStr := sp
		var pos float32
		hasPos := false
		lastSpace := strings.LastIndexByte(sp, ' ')
		if lastSpace > 0 && strings.LastIndexByte(sp, ')') < lastSpace {
			tail := strings.TrimSpace(sp[lastSpace+1:])
			if strings.HasSuffix(tail, "%") {
				if v, err := strconv.ParseFloat(strings.TrimSuffix(tail, "%"), 32); err == nil {
					pos = float32(v / 100.0)
					hasPos = true
					colStr = strings.TrimSpace(sp[:lastSpace])
				}
			}
		}
		c, ok := ParseColor(colStr)
		if !ok {
			fields := strings.Fields(sp)
			if len(fields) > 0 {
				c, ok = ParseColor(fields[0])
			}
		}
		if !ok {
			return LinearGradient{}, false
		}
		parsed = append(parsed, parsedStop{col: c, pos: pos, hasPos: hasPos})
	}
	if len(parsed) < 2 {
		return LinearGradient{}, false
	}
	for idxStop := range parsed {
		if !parsed[idxStop].hasPos {
			parsed[idxStop].pos = float32(idxStop) / float32(len(parsed)-1)
		}
	}
	var stops [4]GradientStop
	count := len(parsed)
	if count > 4 {
		count = 4
	}
	for k := 0; k < count; k++ {
		srcIdx := k
		if len(parsed) > 4 && k == 3 {
			srcIdx = len(parsed) - 1
		}
		stops[k] = GradientStop{Color: parsed[srcIdx].col, Position: parsed[srcIdx].pos}
	}
	return LinearGradient{
		AngleDeg:   angle,
		StartColor: parsed[0].col,
		EndColor:   parsed[len(parsed)-1].col,
		Kind:       kind,
		StopCount:  uint8(count),
		Stops:      stops,
	}, true
}

func parseBoxShadow(raw string, fontSize, rootFont, vw, vh float32) (BoxShadow, bool) {
	if strings.EqualFold(strings.TrimSpace(raw), "none") {
		return BoxShadow{}, false
	}
	fields := strings.Fields(raw)
	if len(fields) < 2 {
		return BoxShadow{}, false
	}
	var lengths []float32
	col := RGBA(0, 0, 0, 80)
	for _, f := range fields {
		if strings.EqualFold(f, "inset") {
			continue
		}
		if c, ok := ParseColor(f); ok {
			col = c
			continue
		}
		lengths = append(lengths, EvalMathPx(f, fontSize, rootFont, vw, vh))
	}
	if len(lengths) < 2 {
		return BoxShadow{}, false
	}
	bs := BoxShadow{OffsetX: lengths[0], OffsetY: lengths[1], Color: col}
	if len(lengths) >= 3 {
		bs.BlurRadius = lengths[2]
	}
	if len(lengths) >= 4 {
		bs.SpreadRadius = lengths[3]
	}
	return bs, true
}

func parseSizeValue(raw string, fontSize, rootFont, vw, vh float32) Size {
	return parseSizeValueWithContainer(raw, fontSize, rootFont, vw, vh, vw, vh)
}

func parseSizeValueWithContainer(raw string, fontSize, rootFont, vw, vh, cqw, cqh float32) Size {
	s := strings.ToLower(strings.TrimSpace(raw))
	if s == "" || s == "auto" || s == "none" {
		return Size{Kind: SizeAuto}
	}
	if strings.HasPrefix(s, "calc(") || strings.HasPrefix(s, "min(") || strings.HasPrefix(s, "max(") || strings.HasPrefix(s, "clamp(") {
		return Size{Kind: SizePx, Value: EvalMathPxWithContainer(s, fontSize, rootFont, vw, vh, cqw, cqh)}
	}
	if strings.HasSuffix(s, "%") {
		v, err := strconv.ParseFloat(strings.TrimSuffix(s, "%"), 32)
		if err == nil {
			return Size{Kind: SizePercent, Value: float32(v)}
		}
	}
	if strings.HasSuffix(s, "cqmin") {
		if v, err := strconv.ParseFloat(strings.TrimSuffix(s, "cqmin"), 32); err == nil {
			return Size{Kind: SizeCqmin, Value: float32(v)}
		}
	}
	if strings.HasSuffix(s, "cqmax") {
		if v, err := strconv.ParseFloat(strings.TrimSuffix(s, "cqmax"), 32); err == nil {
			return Size{Kind: SizeCqmax, Value: float32(v)}
		}
	}
	if strings.HasSuffix(s, "cqw") {
		if v, err := strconv.ParseFloat(strings.TrimSuffix(s, "cqw"), 32); err == nil {
			return Size{Kind: SizeCqw, Value: float32(v)}
		}
	}
	if strings.HasSuffix(s, "cqh") {
		if v, err := strconv.ParseFloat(strings.TrimSuffix(s, "cqh"), 32); err == nil {
			return Size{Kind: SizeCqh, Value: float32(v)}
		}
	}
	if strings.HasSuffix(s, "cqi") {
		if v, err := strconv.ParseFloat(strings.TrimSuffix(s, "cqi"), 32); err == nil {
			return Size{Kind: SizeCqi, Value: float32(v)}
		}
	}
	if strings.HasSuffix(s, "cqb") {
		if v, err := strconv.ParseFloat(strings.TrimSuffix(s, "cqb"), 32); err == nil {
			return Size{Kind: SizeCqb, Value: float32(v)}
		}
	}
	if strings.HasSuffix(s, "rem") {
		v, err := strconv.ParseFloat(strings.TrimSuffix(s, "rem"), 32)
		if err == nil {
			return Size{Kind: SizeRem, Value: float32(v)}
		}
	}
	if strings.HasSuffix(s, "em") {
		v, err := strconv.ParseFloat(strings.TrimSuffix(s, "em"), 32)
		if err == nil {
			return Size{Kind: SizeEm, Value: float32(v)}
		}
	}
	if strings.HasSuffix(s, "vw") {
		v, err := strconv.ParseFloat(strings.TrimSuffix(s, "vw"), 32)
		if err == nil {
			return Size{Kind: SizeVw, Value: float32(v)}
		}
	}
	if strings.HasSuffix(s, "vh") {
		v, err := strconv.ParseFloat(strings.TrimSuffix(s, "vh"), 32)
		if err == nil {
			return Size{Kind: SizeVh, Value: float32(v)}
		}
	}
	if strings.HasSuffix(s, "ch") {
		v, err := strconv.ParseFloat(strings.TrimSuffix(s, "ch"), 32)
		if err == nil {
			return Size{Kind: SizeCh, Value: float32(v)}
		}
	}
	s = strings.TrimSuffix(s, "px")
	v, err := strconv.ParseFloat(s, 32)
	if err == nil {
		return Size{Kind: SizePx, Value: float32(v)}
	}
	return Size{Kind: SizeAuto}
}

func EvalMathPx(raw string, fontSize, rootFont, vw, vh float32) float32 {
	return EvalMathPxWithContainer(raw, fontSize, rootFont, vw, vh, vw, vh)
}

func EvalMathPxWithContainer(raw string, fontSize, rootFont, vw, vh, cqw, cqh float32) float32 {
	s := strings.ToLower(strings.TrimSpace(raw))
	if strings.HasPrefix(s, "calc(") && strings.HasSuffix(s, ")") {
		inner := s[5 : len(s)-1]
		return evalSimpleExprDepth(inner, fontSize, rootFont, vw, vh, cqw, cqh, 0)
	}
	if strings.HasPrefix(s, "min(") && strings.HasSuffix(s, ")") {
		parts := splitSelectors(s[4 : len(s)-1])
		if len(parts) == 0 {
			return 0
		}
		best := EvalMathPxWithContainer(parts[0], fontSize, rootFont, vw, vh, cqw, cqh)
		for _, p := range parts[1:] {
			v := EvalMathPxWithContainer(p, fontSize, rootFont, vw, vh, cqw, cqh)
			if v < best {
				best = v
			}
		}
		return best
	}
	if strings.HasPrefix(s, "max(") && strings.HasSuffix(s, ")") {
		parts := splitSelectors(s[4 : len(s)-1])
		if len(parts) == 0 {
			return 0
		}
		best := EvalMathPxWithContainer(parts[0], fontSize, rootFont, vw, vh, cqw, cqh)
		for _, p := range parts[1:] {
			v := EvalMathPxWithContainer(p, fontSize, rootFont, vw, vh, cqw, cqh)
			if v > best {
				best = v
			}
		}
		return best
	}
	if strings.HasPrefix(s, "clamp(") && strings.HasSuffix(s, ")") {
		parts := splitSelectors(s[6 : len(s)-1])
		if len(parts) == 3 {
			lo := EvalMathPxWithContainer(parts[0], fontSize, rootFont, vw, vh, cqw, cqh)
			val := EvalMathPxWithContainer(parts[1], fontSize, rootFont, vw, vh, cqw, cqh)
			hi := EvalMathPxWithContainer(parts[2], fontSize, rootFont, vw, vh, cqw, cqh)
			return float32(math.Max(float64(lo), math.Min(float64(val), float64(hi))))
		}
	}
	if strings.HasSuffix(s, "cqmin") {
		v, _ := strconv.ParseFloat(strings.TrimSuffix(s, "cqmin"), 32)
		return float32(math.Min(float64(cqw), float64(cqh))) * float32(v) / 100.0
	}
	if strings.HasSuffix(s, "cqmax") {
		v, _ := strconv.ParseFloat(strings.TrimSuffix(s, "cqmax"), 32)
		return float32(math.Max(float64(cqw), float64(cqh))) * float32(v) / 100.0
	}
	if strings.HasSuffix(s, "cqw") || strings.HasSuffix(s, "cqi") {
		v, _ := strconv.ParseFloat(s[:len(s)-3], 32)
		return cqw * float32(v) / 100.0
	}
	if strings.HasSuffix(s, "cqh") || strings.HasSuffix(s, "cqb") {
		v, _ := strconv.ParseFloat(s[:len(s)-3], 32)
		return cqh * float32(v) / 100.0
	}
	if strings.HasSuffix(s, "rem") {
		v, _ := strconv.ParseFloat(strings.TrimSuffix(s, "rem"), 32)
		return float32(v) * rootFont
	}
	if strings.HasSuffix(s, "em") {
		v, _ := strconv.ParseFloat(strings.TrimSuffix(s, "em"), 32)
		return float32(v) * fontSize
	}
	if strings.HasSuffix(s, "vw") {
		v, _ := strconv.ParseFloat(strings.TrimSuffix(s, "vw"), 32)
		return vw * float32(v) / 100.0
	}
	if strings.HasSuffix(s, "vh") {
		v, _ := strconv.ParseFloat(strings.TrimSuffix(s, "vh"), 32)
		return vh * float32(v) / 100.0
	}
	if strings.HasSuffix(s, "ch") {
		v, _ := strconv.ParseFloat(strings.TrimSuffix(s, "ch"), 32)
		return (fontSize * 0.5) * float32(v)
	}
	if strings.HasSuffix(s, "%") {
		v, _ := strconv.ParseFloat(strings.TrimSuffix(s, "%"), 32)
		return vw * float32(v) / 100.0
	}
	s = strings.TrimSuffix(s, "px")
	v, _ := strconv.ParseFloat(s, 32)
	return float32(v)
}

func evalSimpleExpr(expr string, fontSize, rootFont, vw, vh float32) float32 {
	return evalSimpleExprDepth(expr, fontSize, rootFont, vw, vh, vw, vh, 0)
}

func evalSimpleExprDepth(expr string, fontSize, rootFont, vw, vh, cqw, cqh float32, depth int) float32 {
	if depth > 16 {
		return 0
	}
	if idx := strings.Index(expr, " + "); idx > 0 {
		return evalSimpleExprDepth(expr[:idx], fontSize, rootFont, vw, vh, cqw, cqh, depth+1) + evalSimpleExprDepth(expr[idx+3:], fontSize, rootFont, vw, vh, cqw, cqh, depth+1)
	}
	if idx := strings.Index(expr, " - "); idx > 0 {
		return evalSimpleExprDepth(expr[:idx], fontSize, rootFont, vw, vh, cqw, cqh, depth+1) - evalSimpleExprDepth(expr[idx+3:], fontSize, rootFont, vw, vh, cqw, cqh, depth+1)
	}
	if idx := strings.IndexByte(expr, '*'); idx > 0 {
		return evalSimpleExprDepth(expr[:idx], fontSize, rootFont, vw, vh, cqw, cqh, depth+1) * evalSimpleExprDepth(expr[idx+1:], fontSize, rootFont, vw, vh, cqw, cqh, depth+1)
	}
	if idx := strings.IndexByte(expr, '/'); idx > 0 {
		rhs := evalSimpleExprDepth(expr[idx+1:], fontSize, rootFont, vw, vh, cqw, cqh, depth+1)
		if rhs != 0 {
			return evalSimpleExprDepth(expr[:idx], fontSize, rootFont, vw, vh, cqw, cqh, depth+1) / rhs
		}
	}
	return EvalMathPxWithContainer(expr, fontSize, rootFont, vw, vh, cqw, cqh)
}

func resolveCustomVars(val string, props map[string]string, depth int) string {
	if depth > 8 || len(val) > 4096 || !strings.Contains(val, "var(") {
		return val
	}
	idx := strings.Index(val, "var(")
	end := findMatchingParenStr(val, idx+3)
	if end < 0 {
		return val
	}
	inner := val[idx+4 : end]
	varName := inner
	fallback := ""
	if comma := strings.IndexByte(inner, ','); comma >= 0 {
		varName = strings.TrimSpace(inner[:comma])
		fallback = strings.TrimSpace(inner[comma+1:])
	} else {
		varName = strings.TrimSpace(inner)
	}
	replacement, ok := props[varName]
	if !ok || strings.TrimSpace(replacement) == "" {
		replacement = fallback
	}
	next := val[:idx] + replacement + val[end+1:]
	if len(next) > 4096 {
		return val
	}
	return resolveCustomVars(next, props, depth+1)
}

func evalContentValue(d *dom.NodeData, val string) string {
	v := strings.TrimSpace(val)
	if strings.EqualFold(v, "none") || strings.EqualFold(v, "normal") {
		return ""
	}
	if strings.HasPrefix(strings.ToLower(v), "attr(") && strings.HasSuffix(v, ")") {
		attrName := strings.TrimSpace(v[5 : len(v)-1])
		av, _ := d.Attr(attrName)
		return av
	}
	return strings.Trim(v, "\"'")
}

func matchesMediaQuery(mq string, vw, vh uint32) bool {
	s := strings.ToLower(strings.TrimSpace(mq))
	if s == "" || s == "all" || s == "screen" {
		return true
	}
	if strings.Contains(s, "print") && !strings.Contains(s, "not print") {
		return false
	}
	for _, part := range strings.Split(s, ",") {
		part = strings.TrimSpace(part)
		if matchSingleMedia(part, vw, vh) {
			return true
		}
	}
	return false
}

func matchSingleMedia(q string, vw, vh uint32) bool {
	neg := false
	if strings.HasPrefix(q, "not ") {
		neg = true
		q = strings.TrimSpace(q[4:])
	} else if strings.HasPrefix(q, "only ") {
		q = strings.TrimSpace(q[5:])
	}
	res := true
	for _, cond := range strings.Split(q, "and") {
		cond = strings.Trim(strings.TrimSpace(cond), "()")
		if cond == "" || cond == "all" || cond == "screen" {
			continue
		}
		if cond == "print" {
			res = false
			break
		}
		if strings.HasPrefix(cond, "min-width:") {
			px := uint32(EvalMathPx(strings.TrimSpace(cond[10:]), 16, 16, float32(vw), float32(vh)))
			if vw < px {
				res = false
			}
		} else if strings.HasPrefix(cond, "max-width:") {
			px := uint32(EvalMathPx(strings.TrimSpace(cond[10:]), 16, 16, float32(vw), float32(vh)))
			if vw > px {
				res = false
			}
		} else if strings.HasPrefix(cond, "prefers-color-scheme:") {
			scheme := strings.TrimSpace(cond[21:])
			if scheme != "dark" && scheme != "light" {
				res = false
			}
		}
	}
	if neg {
		return !res
	}
	return res
}

func matchesContainerClauses(raw string, containers []containerFrame, vw, vh uint32) bool {
	for _, clause := range strings.Split(raw, ";") {
		clause = strings.TrimSpace(clause)
		if clause == "" {
			continue
		}
		name := ""
		condStr := clause
		if openIdx := strings.IndexByte(clause, '('); openIdx > 0 {
			prefix := strings.ToLower(strings.TrimSpace(clause[:openIdx]))
			if prefix != "not" {
				name = prefix
				condStr = strings.TrimSpace(clause[openIdx:])
			}
		}
		var targetW, targetH uint32 = vw, vh
		found := false
		for i := len(containers) - 1; i >= 0; i-- {
			cf := containers[i]
			if name != "" {
				hasName := false
				for _, cn := range cf.names {
					if cn == name {
						hasName = true
						break
					}
				}
				if !hasName {
					continue
				}
			}
			targetW = uint32(math.Max(0, math.Round(float64(cf.w))))
			targetH = uint32(math.Max(0, math.Round(float64(cf.h))))
			found = true
			break
		}
		if name != "" && !found {
			return false
		}
		if !matchSingleContainerCond(strings.ToLower(condStr), targetW, targetH) {
			return false
		}
	}
	return true
}

func matchSingleContainerCond(q string, cw, ch uint32) bool {
	neg := false
	if strings.HasPrefix(q, "not ") {
		neg = true
		q = strings.TrimSpace(q[4:])
	}
	res := true
	for _, part := range strings.Split(q, "and") {
		c := strings.Trim(strings.TrimSpace(part), "()")
		if c == "" {
			continue
		}
		switch {
		case strings.HasPrefix(c, "min-width:") || strings.HasPrefix(c, "min-inline-size:"):
			colon := strings.IndexByte(c, ':')
			px := uint32(EvalMathPx(strings.TrimSpace(c[colon+1:]), 16, 16, float32(cw), float32(ch)))
			if cw < px {
				res = false
			}
		case strings.HasPrefix(c, "max-width:") || strings.HasPrefix(c, "max-inline-size:"):
			colon := strings.IndexByte(c, ':')
			px := uint32(EvalMathPx(strings.TrimSpace(c[colon+1:]), 16, 16, float32(cw), float32(ch)))
			if cw > px {
				res = false
			}
		case strings.HasPrefix(c, "min-height:") || strings.HasPrefix(c, "min-block-size:"):
			colon := strings.IndexByte(c, ':')
			px := uint32(EvalMathPx(strings.TrimSpace(c[colon+1:]), 16, 16, float32(cw), float32(ch)))
			if ch < px {
				res = false
			}
		case strings.HasPrefix(c, "max-height:") || strings.HasPrefix(c, "max-block-size:"):
			colon := strings.IndexByte(c, ':')
			px := uint32(EvalMathPx(strings.TrimSpace(c[colon+1:]), 16, 16, float32(cw), float32(ch)))
			if ch > px {
				res = false
			}
		case strings.Contains(c, ">="):
			sides := strings.SplitN(c, ">=", 2)
			lhs := strings.TrimSpace(sides[0])
			px := uint32(EvalMathPx(strings.TrimSpace(sides[1]), 16, 16, float32(cw), float32(ch)))
			if lhs == "height" || lhs == "block-size" {
				if ch < px {
					res = false
				}
			} else if cw < px {
				res = false
			}
		case strings.Contains(c, "<="):
			sides := strings.SplitN(c, "<=", 2)
			lhs := strings.TrimSpace(sides[0])
			px := uint32(EvalMathPx(strings.TrimSpace(sides[1]), 16, 16, float32(cw), float32(ch)))
			if lhs == "height" || lhs == "block-size" {
				if ch > px {
					res = false
				}
			} else if cw > px {
				res = false
			}
		case strings.Contains(c, ">"):
			sides := strings.SplitN(c, ">", 2)
			lhs := strings.TrimSpace(sides[0])
			px := uint32(EvalMathPx(strings.TrimSpace(sides[1]), 16, 16, float32(cw), float32(ch)))
			if lhs == "height" || lhs == "block-size" {
				if ch <= px {
					res = false
				}
			} else if cw <= px {
				res = false
			}
		case strings.Contains(c, "<"):
			sides := strings.SplitN(c, "<", 2)
			lhs := strings.TrimSpace(sides[0])
			px := uint32(EvalMathPx(strings.TrimSpace(sides[1]), 16, 16, float32(cw), float32(ch)))
			if lhs == "height" || lhs == "block-size" {
				if ch >= px {
					res = false
				}
			} else if cw >= px {
				res = false
			}
		}
	}
	if neg {
		return !res
	}
	return res
}

func checkScopeProximity(doc *dom.Document, id dom.NodeId, scopeRoot, scopeLimit string) (int, bool) {
	if scopeRoot == "" {
		return 0, true
	}
	cur := id
	hops := 0
	for {
		if scopeLimit != "" && doc.MatchesSelector(cur, scopeLimit) {
			return 0, false
		}
		if doc.MatchesSelector(cur, scopeRoot) {
			return hops, true
		}
		p, ok := doc.Parent(cur)
		if !ok {
			return 0, false
		}
		cur = p
		hops++
	}
}
