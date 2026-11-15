# SPDX-License-Identifier: GPL-2.0-only

## Fuzzy text matching for list search: the query's characters must appear in the text in order
## (case-insensitive, spaces in the query ignored), not necessarily together. Among all the
## ways they can line up, the best scoring one counts: matches at the start of the text or of a
## word, and runs of consecutive matches, score high; letters skipped between matches cost a
## little. "bigint" finds "big-intersection" above "bridge-in-test". O(query × text²) per text,
## for short names in lists of hundreds.
extends RefCounted

const MATCH := 16
const START_BONUS := 10
const WORD_BONUS := 8
const RUN_BONUS := 10
const GAP_OPEN := 3
const GAP_EXTEND := 1
const SEPARATORS := " -_./"
# Score of a cell no alignment reaches.
const NONE := -1000000000

## Scores `text` against `query`: {"score": int, "positions": PackedInt32Array (the matched
## character indices of `text`)}, or an empty dictionary when the query's characters are not
## all in the text in order. An empty query matches everything with score 0.
static func match_text(query: String, text: String) -> Dictionary:
	var needle := query.to_lower().replace(" ", "")
	if needle.is_empty():
		return {"score": 0, "positions": PackedInt32Array()}
	var hay := text.to_lower()
	var m := needle.length()
	var n := hay.length()
	if m > n:
		return {}
	# best[i][j]: best score with needle[i] matched at hay[j]; from[i][j]: where needle[i - 1] sat.
	var best: Array[PackedInt32Array] = []
	var from: Array[PackedInt32Array] = []
	for i in m:
		var row := PackedInt32Array()
		row.resize(n)
		row.fill(NONE)
		best.append(row)
		var back := PackedInt32Array()
		back.resize(n)
		back.fill(-1)
		from.append(back)
	for i in m:
		for j in range(i, n - (m - 1 - i)):
			if hay[j] != needle[i]:
				continue
			var gain := MATCH + _boundary_bonus(text, j)
			if i == 0:
				best[i][j] = gain
				continue
			for k in range(i - 1, j):
				var prev := best[i - 1][k]
				if prev == NONE:
					continue
				var gap := j - k - 1
				var value := prev + gain + (RUN_BONUS if gap == 0 else -(GAP_OPEN + GAP_EXTEND * (gap - 1)))
				if value > best[i][j]:
					best[i][j] = value
					from[i][j] = k
	var end := -1
	for j in n:
		if best[m - 1][j] != NONE and (end < 0 or best[m - 1][j] > best[m - 1][end]):
			end = j
	if end < 0:
		return {}
	var positions := PackedInt32Array()
	positions.resize(m)
	var at := end
	for i in range(m - 1, -1, -1):
		positions[i] = at
		at = from[i][at]
	return {"score": best[m - 1][end], "positions": positions}

# A match where a word starts: the first character, after a separator, a letter after a digit
# (or the reverse) and an upper-case letter after a lower-case one.
static func _boundary_bonus(text: String, j: int) -> int:
	if j == 0:
		return START_BONUS
	var before := text[j - 1]
	var here := text[j]
	if SEPARATORS.contains(before):
		return WORD_BONUS
	if _is_digit(before) != _is_digit(here):
		return WORD_BONUS
	if here != here.to_lower() and before == before.to_lower():
		return WORD_BONUS
	return 0

static func _is_digit(character: String) -> bool:
	return character >= "0" and character <= "9"
