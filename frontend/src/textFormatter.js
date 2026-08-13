// 시각 폭 계산: CJK/한글 = 2, ASCII = 1 (BBS 터미널 기준)
function visualWidth(str) {
    let w = 0;
    for (const ch of str) {
        const cp = ch.codePointAt(0);
        if (
            (cp >= 0x1100 && cp <= 0x115F) ||   // Hangul Jamo
            (cp >= 0x2E80 && cp <= 0x303E) ||   // CJK Radicals
            (cp >= 0x3041 && cp <= 0x33BF) ||   // Hiragana, Katakana, CJK
            (cp >= 0x3400 && cp <= 0x4DBF) ||   // CJK Extension A
            (cp >= 0x4E00 && cp <= 0x9FFF) ||   // CJK Unified Ideographs
            (cp >= 0xAC00 && cp <= 0xD7AF) ||   // Hangul Syllables
            (cp >= 0xF900 && cp <= 0xFAFF) ||   // CJK Compatibility
            (cp >= 0xFE30 && cp <= 0xFE4F) ||   // CJK Compatibility Forms
            (cp >= 0xFF01 && cp <= 0xFF60) ||   // Fullwidth Forms
            (cp >= 0xFFE0 && cp <= 0xFFE6) ||   // Fullwidth Signs
            (cp >= 0x20000 && cp <= 0x2FA1F)    // CJK Extension B+
        ) {
            w += 2;
        } else {
            w += 1;
        }
    }
    return w;
}

// 앞뒤 공백 제거 (들여쓰기 + 후행공백)
export function trimLines(lines) {
    return lines.map(line => line.trim());
}

// ---계속---/---끝--- 마커 제거 (trimLines 이후 호출 — 줄이 이미 trim됨)
export function stripContinuationMarkers(lines) {
    return lines.filter(line => !/^-{3,}(계속|끝)-{3,}$/.test(line));
}

// 이중행간 감지: 내용줄 중 70%+가 직후 빈줄을 가지면 이중행간
function isDoubleSpaced(lines) {
    let contentLines = 0;
    let followedByBlank = 0;
    for (let i = 0; i < lines.length; i++) {
        if (lines[i] === '') continue;
        contentLines++;
        if (i + 1 < lines.length && lines[i + 1] === '') {
            followedByBlank++;
        }
    }
    if (contentLines < 5) return false;
    return followedByBlank / contentLines >= 0.7;
}

// 이중행간 축소: 빈줄 1개(행간 노이즈) → 제거, 빈줄 2개+(진짜 단락) → 빈줄 1개
export function collapseDoubleSpacing(lines) {
    if (!isDoubleSpaced(lines)) return lines;

    const result = [];
    let blankCount = 0;
    for (const line of lines) {
        if (line === '') {
            blankCount++;
            continue;
        }
        if (blankCount >= 2) {
            result.push('');
        }
        // blankCount === 1 → 행간 노이즈, 제거 (push 안 함)
        blankCount = 0;
        result.push(line);
    }
    return result;
}

// 자동 폭 감지: "긴 줄"(시각 폭 55 이상)의 최빈 폭 탐지
function detectWrapWidth(lines) {
    const MIN_LONG = 55;
    const bucketSize = 2;
    const buckets = new Map();
    let totalLong = 0;

    for (const line of lines) {
        if (line === '') continue;
        const vw = visualWidth(line);
        if (vw < MIN_LONG) continue;
        totalLong++;
        const bucket = Math.floor(vw / bucketSize) * bucketSize;
        buckets.set(bucket, (buckets.get(bucket) || 0) + 1);
    }

    if (buckets.size === 0) return 0;

    let maxCount = 0;
    let maxBucket = 0;
    for (const [bucket, count] of buckets) {
        if (count > maxCount) {
            maxCount = count;
            maxBucket = bucket;
        }
    }

    // 최빈폭 검증: 최빈 버킷이 전체 긴줄의 15% 미만이면 폭감지 실패
    if (totalLong > 0 && maxCount / totalLong < 0.15) return 0;

    return maxBucket;
}

// 줄이 문장종결 부호로 끝나는지 확인
function endsWithTerminator(line) {
    /* v8 ignore next -- unwrapForceBreaks calls this helper with trimmed non-empty lines. */
    if (line.length === 0) return false;
    const last = line[line.length - 1];
    return '.!?」"\u2019\'』'.includes(last);
}

// 줄이 대화 시작인지 확인
function startsWithDialogue(line) {
    /* v8 ignore next -- unwrapForceBreaks calls this helper with trimmed non-empty lines. */
    if (line.length === 0) return false;
    return '"\u201C「『'.includes(line[0]);
}

// 강제 줄넘김 해제 (trimLines 이후 호출 — 줄이 이미 trim됨)
export function unwrapForceBreaks(lines, options = {}) {
    const { trimJoinSpaces = false } = options;
    const wrapWidth = detectWrapWidth(lines);
    if (wrapWidth === 0) return lines;

    const threshold = wrapWidth - 8;
    const upperBound = wrapWidth + 8;
    const result = [];
    let i = 0;

    while (i < lines.length) {
        const line = lines[i];

        if (line === '') {
            result.push(line);
            i++;
            continue;
        }

        let merged = line;
        while (i + 1 < lines.length) {
            const next = lines[i + 1];

            if (next === '') break;
            if (startsWithDialogue(next)) break;

            const currentVw = visualWidth(lines[i]);

            // 상한선: 이미 합쳐진 줄이면 합치기 중단
            if (currentVw > upperBound) break;

            // 시각 폭이 threshold 미만이면 강제 줄넘김이 아님 → 중단
            if (currentVw < threshold) break;

            // 현재 줄이 문장종결 부호로 끝나면서 짧은 경우 중단
            if (endsWithTerminator(lines[i]) && currentVw < wrapWidth - 4) break;

            const separator = trimJoinSpaces ? '' : ' ';
            merged = merged + separator + next;
            i++;
        }
        result.push(merged);
        i++;
    }
    return result;
}

// 구분선 감지 (trimLines 이후 호출 — 줄이 이미 trim됨)
function isSeparatorLine(line) {
    return /^[*]{3,}$/.test(line) ||
           /^-{3,}$/.test(line) ||
           /^={3,}$/.test(line) ||
           /^[*\s]{5,}$/.test(line) ||
           (/^[-\s]{5,}$/.test(line) && line.replace(/\s/g, '').length >= 3);
}

// 제목 감지: 15자 미만 단독 줄 (앞뒤가 빈 줄), 문장종결 부호로 끝나면 제외
function isHeaderCandidate(line) {
    if (line.length === 0 || line.length >= 15) return false;
    return !endsWithTerminator(line);
}

// 단락 구조화 (trimLines 이후 호출 — 줄이 이미 trim됨)
export function buildParagraphs(lines, minBlankLines = 1) {
    const blocks = [];
    let current = [];
    let blankCount = 0;

    function flushCurrent() {
        if (current.length === 0) return;
        const text = current.join(' ');
        const firstLine = current[0];
        const type = startsWithDialogue(firstLine) ? 'dialogue' : 'narrative';
        blocks.push({ type, text });
        current = [];
    }

    for (let i = 0; i < lines.length; i++) {
        const line = lines[i];

        if (line === '') {
            blankCount++;
            continue;
        }

        if (isSeparatorLine(line)) {
            flushCurrent();
            blocks.push({ type: 'separator', text: line });
            blankCount = 0;
            continue;
        }

        if (current.length > 0 && blankCount >= minBlankLines) {
            flushCurrent();
        }

        if (current.length > 0 && startsWithDialogue(line)) {
            flushCurrent();
        }

        if (blankCount > 0 && current.length === 0 && !startsWithDialogue(line) && isHeaderCandidate(line)) {
            const nextIsBlank = (i + 1 >= lines.length) || lines[i + 1] === '';
            if (nextIsBlank) {
                flushCurrent();
                blocks.push({ type: 'header', text: line });
                blankCount = 0;
                continue;
            }
        }

        blankCount = 0;
        current.push(line);
    }
    flushCurrent();
    return blocks;
}

// 빈 단락 제거
export function trimEmptyParagraphs(blocks) {
    return blocks.filter(b => b.text.trim() !== '');
}

// 통합 파이프라인
export function formatText(lines, options = {}) {
    const { minBlankLines = 1 } = options;

    let processed = trimLines(lines);
    processed = stripContinuationMarkers(processed);
    processed = collapseDoubleSpacing(processed);
    const blocks = buildParagraphs(processed, minBlankLines);
    return trimEmptyParagraphs(blocks);
}

export { visualWidth, detectWrapWidth, isDoubleSpaced };
