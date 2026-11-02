// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

package anvilttf

import "math/bits"

// parseKern decodes version 0 format 0 kerning pairs.
func parseKern(kern []byte) (map[[2]uint16]int16, error) {
	if len(kern) < 4 {
		return nil, errInvalidTable("kern")
	}

	version, err := readU16BE(kern, 0)
	if err != nil || version != 0 {
		return make(map[[2]uint16]int16), nil
	}

	nTablesRaw, err := readU16BE(kern, 2)
	if err != nil {
		return make(map[[2]uint16]int16), nil
	}
	nTables := int(nTablesRaw)

	pos := 4
	pairs := make(map[[2]uint16]int16)

	for t := 0; t < nTables; t++ {
		if pos+6 > len(kern) {
			break
		}
		subLenRaw, err := readU16BE(kern, pos+2)
		if err != nil {
			break
		}
		subLen := int(subLenRaw)
		coverage, err := readU16BE(kern, pos+4)
		if err != nil {
			break
		}

		format := coverage >> 8
		if format == 0 && pos+14 <= len(kern) {
			nPairsRaw, err := readU16BE(kern, pos+6)
			if err == nil {
				nPairs := int(nPairsRaw)
				p := pos + 14
				for i := 0; i < nPairs; i++ {
					if p+6 > len(kern) {
						break
					}
					left, err1 := readU16BE(kern, p)
					right, err2 := readU16BE(kern, p+2)
					val, err3 := readI16BE(kern, p+4)
					if err1 == nil && err2 == nil && err3 == nil {
						pairs[[2]uint16{left, right}] = val
					}
					p += 6
				}
			}
		}

		if subLen == 0 {
			break
		}
		pos += subLen
	}

	return pairs, nil
}

func valueRecordByteLen(format uint16) int {
	return bits.OnesCount16(format) * 2
}

func xAdvanceOffsetInRecord(format uint16) (int, bool) {
	if (format & 0x0004) == 0 {
		return 0, false
	}
	earlierFields := bits.OnesCount16(format & 0x0003)
	return earlierFields * 2, true
}

func parseGposCoverage(cov []byte) [][2]uint16 {
	var out [][2]uint16
	if len(cov) < 4 {
		return out
	}
	format, err := readU16BE(cov, 0)
	if err != nil {
		return out
	}

	if format == 1 {
		countRaw, err := readU16BE(cov, 2)
		if err != nil {
			return out
		}
		n := int(countRaw)
		if n > 4096 {
			n = 4096
		}
		if 4+n*2 > len(cov) {
			return out
		}
		for i := 0; i < n; i++ {
			if gid, err := readU16BE(cov, 4+i*2); err == nil {
				out = append(out, [2]uint16{uint16(i), gid})
			}
		}
	} else if format == 2 {
		rangeCountRaw, err := readU16BE(cov, 2)
		if err != nil {
			return out
		}
		n := int(rangeCountRaw)
		if n > 1024 {
			n = 1024
		}
		if 4+n*6 > len(cov) {
			return out
		}
		for i := 0; i < n; i++ {
			rec := 4 + i*6
			startGid, err1 := readU16BE(cov, rec)
			endGid, err2 := readU16BE(cov, rec+2)
			startCov, err3 := readU16BE(cov, rec+4)
			if err1 != nil || err2 != nil || err3 != nil || endGid < startGid || (endGid-startGid) > 2048 {
				continue
			}
			for gid := startGid; gid <= endGid; gid++ {
				covIdx := startCov + (gid - startGid)
				out = append(out, [2]uint16{covIdx, gid})
			}
		}
	}

	return out
}

func parseGposClassDef(cls []byte) map[uint16]uint16 {
	m := make(map[uint16]uint16)
	if len(cls) < 4 {
		return m
	}
	format, err := readU16BE(cls, 0)
	if err != nil {
		return m
	}

	if format == 1 {
		startGid, err1 := readU16BE(cls, 2)
		countRaw, err2 := readU16BE(cls, 4)
		if err1 != nil || err2 != nil {
			return m
		}
		n := int(countRaw)
		if n > 4096 {
			n = 4096
		}
		if 6+n*2 > len(cls) {
			return m
		}
		for i := 0; i < n; i++ {
			if c, err := readU16BE(cls, 6+i*2); err == nil && c != 0 {
				m[startGid+uint16(i)] = c
			}
		}
	} else if format == 2 {
		rangeCountRaw, err := readU16BE(cls, 2)
		if err != nil {
			return m
		}
		n := int(rangeCountRaw)
		if n > 1024 {
			n = 1024
		}
		if 4+n*6 > len(cls) {
			return m
		}
		for i := 0; i < n; i++ {
			rec := 4 + i*6
			startGid, err1 := readU16BE(cls, rec)
			endGid, err2 := readU16BE(cls, rec+2)
			classVal, err3 := readU16BE(cls, rec+4)
			if err1 != nil || err2 != nil || err3 != nil || classVal == 0 || endGid < startGid || (endGid-startGid) > 2048 {
				continue
			}
			for gid := startGid; gid <= endGid; gid++ {
				m[gid] = classVal
			}
		}
	}

	return m
}

func parseGposPairPosSubtable(sub []byte, out map[[2]uint16]int16) {
	if len(sub) < 10 {
		return
	}
	posFormat, err := readU16BE(sub, 0)
	if err != nil {
		return
	}

	if posFormat == 1 {
		covOffRaw, err1 := readU16BE(sub, 2)
		vf1, err2 := readU16BE(sub, 4)
		vf2, err3 := readU16BE(sub, 6)
		pairSetCountRaw, err4 := readU16BE(sub, 8)
		if err1 != nil || err2 != nil || err3 != nil || err4 != nil {
			return
		}

		xAdvOff, ok := xAdvanceOffsetInRecord(vf1)
		if !ok {
			return
		}
		covOff := int(covOffRaw)
		psCount := int(pairSetCountRaw)
		if psCount > 4096 {
			psCount = 4096
		}
		if covOff >= len(sub) || 10+psCount*2 > len(sub) {
			return
		}

		recLen := 2 + valueRecordByteLen(vf1) + valueRecordByteLen(vf2)
		coverage := parseGposCoverage(sub[covOff:])
		for _, pair := range coverage {
			covIdx := int(pair[0])
			leftGid := pair[1]
			if covIdx >= psCount {
				continue
			}
			psOffRaw, err := readU16BE(sub, 10+covIdx*2)
			if err != nil {
				continue
			}
			psOff := int(psOffRaw)
			if psOff+2 > len(sub) {
				continue
			}
			pvCountRaw, err := readU16BE(sub, psOff)
			if err != nil {
				continue
			}
			pvCount := int(pvCountRaw)
			if pvCount > 2048 {
				pvCount = 2048
			}

			p := psOff + 2
			for i := 0; i < pvCount; i++ {
				if p+recLen > len(sub) {
					break
				}
				rightGid, err1 := readU16BE(sub, p)
				xAdv, err2 := readI16BE(sub, p+2+xAdvOff)
				if err1 == nil && err2 == nil && xAdv != 0 {
					out[[2]uint16{leftGid, rightGid}] = xAdv
				}
				p += recLen
			}
		}
	} else if posFormat == 2 {
		if len(sub) < 16 {
			return
		}
		covOffRaw, err1 := readU16BE(sub, 2)
		vf1, err2 := readU16BE(sub, 4)
		vf2, err3 := readU16BE(sub, 6)
		cd1OffRaw, err4 := readU16BE(sub, 8)
		cd2OffRaw, err5 := readU16BE(sub, 10)
		c1CntRaw, err6 := readU16BE(sub, 12)
		c2CntRaw, err7 := readU16BE(sub, 14)
		if err1 != nil || err2 != nil || err3 != nil || err4 != nil || err5 != nil || err6 != nil || err7 != nil {
			return
		}

		xAdvOff, ok := xAdvanceOffsetInRecord(vf1)
		if !ok {
			return
		}
		covOff := int(covOffRaw)
		cd1Off := int(cd1OffRaw)
		cd2Off := int(cd2OffRaw)
		if covOff >= len(sub) || cd1Off >= len(sub) || cd2Off >= len(sub) {
			return
		}

		c1Cnt := int(c1CntRaw)
		if c1Cnt > 512 {
			c1Cnt = 512
		}
		c2Cnt := int(c2CntRaw)
		if c2Cnt > 512 {
			c2Cnt = 512
		}

		cellLen := valueRecordByteLen(vf1) + valueRecordByteLen(vf2)
		if cellLen == 0 || 16+c1Cnt*c2Cnt*cellLen > len(sub) {
			return
		}

		coverage := parseGposCoverage(sub[covOff:])
		class1 := parseGposClassDef(sub[cd1Off:])
		class2 := parseGposClassDef(sub[cd2Off:])

		class2ByID := make(map[uint16][]uint16)
		for gid, cid := range class2 {
			class2ByID[cid] = append(class2ByID[cid], gid)
		}

		for _, pair := range coverage {
			leftGid := pair[1]
			c1 := int(class1[leftGid])
			if c1 >= c1Cnt {
				continue
			}
			for c2 := 1; c2 < c2Cnt; c2++ {
				cellOff := 16 + (c1*c2Cnt+c2)*cellLen
				xAdv, err := readI16BE(sub, cellOff+xAdvOff)
				if err != nil || xAdv == 0 {
					continue
				}
				if rightGids, exists := class2ByID[uint16(c2)]; exists {
					for _, rightGid := range rightGids {
						out[[2]uint16{leftGid, rightGid}] = xAdv
					}
				}
			}
		}
	}
}

func parseGpos(gpos []byte, out map[[2]uint16]int16) {
	if len(gpos) < 10 {
		return
	}
	major, err1 := readU16BE(gpos, 0)
	minor, err2 := readU16BE(gpos, 2)
	llOffRaw, err3 := readU16BE(gpos, 8)
	if err1 != nil || err2 != nil || err3 != nil || major != 1 || (minor != 0 && minor != 1) {
		return
	}

	llOff := int(llOffRaw)
	if llOff+2 > len(gpos) {
		return
	}
	lookupCountRaw, err := readU16BE(gpos, llOff)
	if err != nil {
		return
	}
	nLookups := int(lookupCountRaw)
	if nLookups > 256 {
		nLookups = 256
	}
	if llOff+2+nLookups*2 > len(gpos) {
		return
	}

	for i := 0; i < nLookups; i++ {
		lRelRaw, err := readU16BE(gpos, llOff+2+i*2)
		if err != nil {
			continue
		}
		lOff := llOff + int(lRelRaw)
		if lOff+6 > len(gpos) {
			continue
		}
		lookupType, err1 := readU16BE(gpos, lOff)
		subCountRaw, err2 := readU16BE(gpos, lOff+4)
		if err1 != nil || err2 != nil {
			continue
		}

		nSubs := int(subCountRaw)
		if nSubs > 128 {
			nSubs = 128
		}
		if lOff+6+nSubs*2 > len(gpos) {
			continue
		}

		for s := 0; s < nSubs; s++ {
			subRelRaw, err := readU16BE(gpos, lOff+6+s*2)
			if err != nil {
				continue
			}
			subOff := lOff + int(subRelRaw)
			if subOff >= len(gpos) {
				continue
			}

			if lookupType == 2 {
				parseGposPairPosSubtable(gpos[subOff:], out)
			} else if lookupType == 9 && subOff+8 <= len(gpos) {
				extFmt, err1 := readU16BE(gpos, subOff)
				extType, err2 := readU16BE(gpos, subOff+2)
				extOffRaw, err3 := readU32BE(gpos, subOff+4)
				if err1 == nil && err2 == nil && err3 == nil && extFmt == 1 && extType == 2 {
					target := subOff + int(extOffRaw)
					if target < len(gpos) {
						parseGposPairPosSubtable(gpos[target:], out)
					}
				}
			}
		}
	}
}
