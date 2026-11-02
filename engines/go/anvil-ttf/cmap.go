// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anvilttf

// parseCmap parses the cmap table supporting format 4 (Unicode BMP) and format 12 (UCS-4).
func parseCmap(cmap []byte) (map[rune]uint16, error) {
	if len(cmap) < 4 {
		return nil, errInvalidTable("cmap")
	}

	numSubtablesRaw, err := readU16BE(cmap, 2)
	if err != nil {
		return nil, err
	}
	numSubtables := int(numSubtablesRaw)
	if 4+numSubtables*8 > len(cmap) {
		return nil, errInvalidTable("cmap")
	}

	out := make(map[rune]uint16)
	for i := 0; i < numSubtables; i++ {
		rec := 4 + i*8
		platformID, err := readU16BE(cmap, rec)
		if err != nil {
			continue
		}
		encodingID, err := readU16BE(cmap, rec+2)
		if err != nil {
			continue
		}
		subOffsetRaw, err := readU32BE(cmap, rec+4)
		if err != nil {
			continue
		}
		subOffset := int(subOffsetRaw)
		if subOffset < 0 || subOffset+4 > len(cmap) {
			continue
		}

		// Platform 0: Unicode. Platform 3: Windows (Encoding 1: BMP, Encoding 10: Full Unicode).
		if platformID != 0 && !(platformID == 3 && (encodingID == 1 || encodingID == 10)) {
			continue
		}

		sub := cmap[subOffset:]
		format, err := readU16BE(sub, 0)
		if err != nil {
			continue
		}

		switch format {
		case 4:
			_ = parseCmapFormat4(sub, out)
		case 12:
			_ = parseCmapFormat12(sub, out)
		}
	}

	return out, nil
}

// parseCmapFormat4 parses a format-4 segmented BMP subtable.
func parseCmapFormat4(sub []byte, out map[rune]uint16) error {
	if len(sub) < 14 {
		return errInvalidTable("cmap")
	}

	segCountX2, err := readU16BE(sub, 6)
	if err != nil {
		return err
	}
	segCount := int(segCountX2) / 2
	if segCount <= 0 {
		return nil
	}

	endCodes := 14
	startCodes := endCodes + segCount*2 + 2 // +2 skips the reserved pad
	idDeltas := startCodes + segCount*2
	idRangeOffsets := idDeltas + segCount*2

	if idRangeOffsets+segCount*2 > len(sub) {
		return errInvalidTable("cmap")
	}

	for seg := 0; seg < segCount; seg++ {
		endCode, err := readU16BE(sub, endCodes+seg*2)
		if err != nil {
			break
		}
		startCode, err := readU16BE(sub, startCodes+seg*2)
		if err != nil {
			break
		}
		idDelta, err := readI16BE(sub, idDeltas+seg*2)
		if err != nil {
			break
		}
		idRangePos := idRangeOffsets + seg*2
		idRangeOffsetRaw, err := readU16BE(sub, idRangePos)
		if err != nil {
			break
		}
		idRangeOffset := int(idRangeOffsetRaw)

		// 0xFFFF sentinel ends the table.
		if startCode == 0xFFFF && endCode == 0xFFFF {
			break
		}
		if startCode > endCode {
			continue
		}

		for code := int(startCode); code <= int(endCode); code++ {
			var gid uint16
			if idRangeOffset == 0 {
				gid = uint16(int32(code) + int32(idDelta))
			} else {
				glyphOff := idRangePos + idRangeOffset + (code-int(startCode))*2
				if glyphOff < 0 || glyphOff+2 > len(sub) {
					continue
				}
				rawGid, err := readU16BE(sub, glyphOff)
				if err != nil || rawGid == 0 {
					gid = 0
				} else {
					gid = uint16(int32(rawGid) + int32(idDelta))
				}
			}

			if gid != 0 {
				out[rune(code)] = gid
			}
		}
	}

	return nil
}

// parseCmapFormat12 parses a format-12 segmented coverage (UCS-4) subtable.
func parseCmapFormat12(sub []byte, out map[rune]uint16) error {
	if len(sub) < 16 {
		return errInvalidTable("cmap")
	}

	numGroupsRaw, err := readU32BE(sub, 12)
	if err != nil {
		return err
	}
	numGroups := int(numGroupsRaw)
	if numGroups < 0 || 16+numGroups*12 > len(sub) {
		return errInvalidTable("cmap")
	}

	for i := 0; i < numGroups; i++ {
		base := 16 + i*12
		startCode, err := readU32BE(sub, base)
		if err != nil {
			break
		}
		endCode, err := readU32BE(sub, base+4)
		if err != nil {
			break
		}
		startGid, err := readU32BE(sub, base+8)
		if err != nil {
			break
		}

		if endCode < startCode || (endCode-startCode) > 0xFFFF {
			continue
		}

		for code := startCode; code <= endCode; code++ {
			gid := uint16(startGid + (code - startCode))
			if gid != 0 {
				out[rune(code)] = gid
			}
		}
	}

	return nil
}
