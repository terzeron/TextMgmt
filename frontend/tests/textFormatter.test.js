import { describe, it, expect } from 'vitest';
import {
    trimLines,
    stripContinuationMarkers,
    collapseDoubleSpacing,
    unwrapForceBreaks,
    buildParagraphs,
    trimEmptyParagraphs,
    formatText,
    visualWidth,
    detectWrapWidth,
    isDoubleSpaced,
} from '../src/textFormatter';

describe('visualWidth', () => {
    it('ASCII 문자는 폭 1', () => {
        expect(visualWidth('hello')).toBe(5);
    });

    it('한글 문자는 폭 2', () => {
        expect(visualWidth('가나다')).toBe(6);
    });

    it('혼합 문자열의 시각 폭을 정확히 계산', () => {
        expect(visualWidth('Hello 세계')).toBe(10);
    });
});

describe('trimLines', () => {
    it('앞뒤 공백을 모두 제거한다', () => {
        const lines = ['  안녕하세요   ', '    세계  ', '끝'];
        expect(trimLines(lines)).toEqual(['안녕하세요', '세계', '끝']);
    });

    it('들여쓰기(10칸)를 제거한다', () => {
        const lines = ['          극악서생 내용'];
        expect(trimLines(lines)).toEqual(['극악서생 내용']);
    });

    it('빈 줄은 빈 문자열로 유지', () => {
        const lines = ['내용', '   ', '다음'];
        expect(trimLines(lines)).toEqual(['내용', '', '다음']);
    });
});

describe('stripContinuationMarkers', () => {
    it('---계속--- 마커를 제거한다', () => {
        const lines = ['본문 내용', '---계속---', '다음 내용'];
        expect(stripContinuationMarkers(lines)).toEqual(['본문 내용', '다음 내용']);
    });

    it('---끝--- 마커를 제거한다', () => {
        const lines = ['본문', '---끝---', '다음'];
        expect(stripContinuationMarkers(lines)).toEqual(['본문', '다음']);
    });

    it('다양한 대시 길이의 마커를 제거한다', () => {
        const lines = ['본문', '----계속----', '------끝------', '다음'];
        expect(stripContinuationMarkers(lines)).toEqual(['본문', '다음']);
    });

    it('일반 구분선(---)은 유지한다', () => {
        const lines = ['본문', '---', '다음'];
        expect(stripContinuationMarkers(lines)).toEqual(['본문', '---', '다음']);
    });

    it('마커가 없으면 원본 그대로 반환', () => {
        const lines = ['가', '나', '다'];
        expect(stripContinuationMarkers(lines)).toEqual(lines);
    });
});

describe('isDoubleSpaced / collapseDoubleSpacing', () => {
    it('이중행간 파일을 감지한다', () => {
        // 내용줄 10개, 각각 뒤에 빈줄 → 100% followedByBlank
        const lines = [];
        for (let i = 0; i < 10; i++) {
            lines.push(`내용 ${i}`);
            lines.push('');
        }
        expect(isDoubleSpaced(lines)).toBe(true);
    });

    it('단일행간 파일은 이중행간이 아니다', () => {
        const lines = [
            '첫 줄', '둘째 줄', '셋째 줄', '넷째 줄', '다섯째 줄',
            '', '새 단락', '계속', '이어짐', '마지막',
        ];
        expect(isDoubleSpaced(lines)).toBe(false);
    });

    it('내용줄이 5개 미만이면 이중행간으로 판정하지 않는다', () => {
        const lines = ['가', '', '나', '', '다', ''];
        expect(isDoubleSpaced(lines)).toBe(false);
    });

    it('이중행간 축소: 빈줄 1개 → 제거, 빈줄 2개+ → 빈줄 1개', () => {
        const lines = [];
        // 단락 1: 3줄 (이중행간)
        lines.push('가', '', '나', '', '다', '');
        // 단락 구분: 빈줄 2개 (원래 빈줄 4개 = 이중행간 2빈줄)
        lines.push('', '', '');
        // 단락 2: 3줄 (이중행간)
        lines.push('라', '', '마', '', '바', '');

        const result = collapseDoubleSpacing(lines);
        // 단락 내 행간 빈줄 제거, 단락 간 빈줄 2개+ → 1개
        expect(result).toEqual(['가', '나', '다', '', '라', '마', '바']);
    });

    it('비이중행간 파일은 그대로 통과', () => {
        const lines = ['가', '나', '다', '', '라', '마', '바'];
        expect(collapseDoubleSpacing(lines)).toEqual(lines);
    });
});

describe('detectWrapWidth', () => {
    it('일관된 폭의 줄에서 최빈폭을 감지한다', () => {
        const lines = Array(20).fill('가'.repeat(36)); // 폭 72
        const width = detectWrapWidth(lines);
        expect(width).toBe(72);
    });

    it('짧은 줄만 있으면 0 반환', () => {
        const lines = ['짧은 줄', '역시 짧음'];
        expect(detectWrapWidth(lines)).toBe(0);
    });

    it('최빈 버킷이 15% 미만이면 0 반환 (폭감지 실패)', () => {
        // 다양한 폭의 줄을 골고루 분산시켜 최빈 버킷 비율이 낮게
        const lines = [];
        for (let w = 55; w <= 200; w += 2) {
            const charCount = Math.floor(w / 2);
            lines.push('가'.repeat(charCount));
        }
        expect(detectWrapWidth(lines)).toBe(0);
    });
});

describe('unwrapForceBreaks', () => {
    function makeLongLines(count, width) {
        const charCount = Math.floor(width / 2);
        const line = '가'.repeat(charCount);
        return Array(count).fill(line);
    }

    it('강제 줄넘김으로 잘린 줄을 합친다', () => {
        const longLine = '가'.repeat(36); // 폭 72
        const padding = makeLongLines(10, 72);
        const lines = [...padding, '', longLine, '나머지 텍스트'];
        const result = unwrapForceBreaks(lines);
        const joined = result.join('\n');
        expect(joined).toContain(longLine + ' 나머지 텍스트');
    });

    it('빈 줄에서는 합치기를 중단한다', () => {
        const padding = makeLongLines(10, 72);
        const lines = [...padding, '', '첫 단락 끝', '', '둘째 단락'];
        const result = unwrapForceBreaks(lines);
        expect(result.filter(l => l === '').length).toBeGreaterThan(0);
    });

    it('대화 시작 줄 앞에서 합치기를 중단한다', () => {
        const longLine = '가'.repeat(36); // 폭 72
        const padding = makeLongLines(10, 72);
        const lines = [...padding, '', longLine, '"이것은 대화입니다"'];
        const result = unwrapForceBreaks(lines);
        expect(result.some(l => l.startsWith('"이것은'))).toBe(true);
    });

    it('ASCII 쌍따옴표도 대화로 인식한다', () => {
        const longLine = '가'.repeat(36);
        const padding = makeLongLines(10, 72);
        const lines = [...padding, '', longLine, '"이것은 대화입니다"'];
        const result = unwrapForceBreaks(lines);
        expect(result.some(l => l.startsWith('"이것은'))).toBe(true);
    });

    it('폭 감지 불가 시 원본 유지', () => {
        const lines = ['짧은 줄', '역시 짧음'];
        expect(unwrapForceBreaks(lines)).toEqual(lines);
    });

    it('trimJoinSpaces=true이면 공백 없이 합친다', () => {
        const padding = makeLongLines(10, 72);
        const longLine = '가'.repeat(36);
        const lines = [...padding, '', longLine, '계속되는부분'];
        const result = unwrapForceBreaks(lines, { trimJoinSpaces: true });
        const joined = result.join('\n');
        expect(joined).toContain(longLine + '계속되는부분');
    });

    it('trimJoinSpaces=false(기본값)이면 공백으로 합친다', () => {
        const padding = makeLongLines(10, 72);
        const longLine = '가'.repeat(36);
        const lines = [...padding, '', longLine, '계속되는부분'];
        const result = unwrapForceBreaks(lines, { trimJoinSpaces: false });
        const joined = result.join('\n');
        expect(joined).toContain(longLine + ' 계속되는부분');
    });

    it('상한선: 이미 합쳐진 긴 줄은 추가 합치기 안 함', () => {
        const padding = makeLongLines(10, 72);
        // wrapWidth=72, upperBound=80 — 폭 90인 줄은 상한선 초과
        const veryLongLine = '가'.repeat(45); // 폭 90
        const lines = [...padding, '', veryLongLine, '추가 텍스트'];
        const result = unwrapForceBreaks(lines);
        // veryLongLine이 추가 텍스트와 합쳐지지 않아야 함
        expect(result.some(l => l === veryLongLine)).toBe(true);
    });
});

describe('buildParagraphs', () => {
    it('빈 줄 기준으로 단락을 분리한다', () => {
        const lines = ['첫 번째 단락 첫 줄', '첫 번째 단락 둘째 줄', '', '두 번째 단락'];
        const blocks = buildParagraphs(lines, 1);
        expect(blocks).toHaveLength(2);
        expect(blocks[0].text).toContain('첫 번째 단락');
        expect(blocks[1].text).toBe('두 번째 단락');
    });

    it('대화를 별도 단락으로 분리한다', () => {
        const lines = ['서술 부분입니다.', '\u201C이것은 대화입니다.\u201D', '다시 서술.'];
        const blocks = buildParagraphs(lines, 1);
        expect(blocks.some(b => b.type === 'dialogue')).toBe(true);
    });

    it('ASCII 쌍따옴표 대화를 감지한다', () => {
        const lines = ['서술 부분입니다.', '"이것은 대화입니다."', '다시 서술.'];
        const blocks = buildParagraphs(lines, 1);
        expect(blocks.some(b => b.type === 'dialogue')).toBe(true);
    });

    it('구분선을 감지한다', () => {
        const lines = ['내용', '', '***', '', '다음 내용'];
        const blocks = buildParagraphs(lines, 1);
        expect(blocks.some(b => b.type === 'separator')).toBe(true);
    });

    it('제목을 감지한다', () => {
        const lines = ['', '제1장', '', '본문이 시작됩니다.'];
        const blocks = buildParagraphs(lines, 1);
        expect(blocks.some(b => b.type === 'header')).toBe(true);
        expect(blocks.find(b => b.type === 'header').text).toBe('제1장');
    });

    it('문장종결 부호로 끝나는 짧은 줄은 제목이 아니다', () => {
        const lines = ['', '그래.', '', '다음 내용이 이어지는 긴 문장입니다.'];
        const blocks = buildParagraphs(lines, 1);
        // '그래.'는 문장종결 부호로 끝나므로 header가 아닌 narrative
        const first = blocks[0];
        expect(first.type).toBe('narrative');
        expect(first.text).toBe('그래.');
    });

    it('문장종결 부호 없는 짧은 줄은 제목으로 감지한다', () => {
        const lines = ['', '제1장', '', '본문이 시작됩니다.'];
        const blocks = buildParagraphs(lines, 1);
        expect(blocks.find(b => b.type === 'header').text).toBe('제1장');
    });

    it('minBlankLines 설정을 반영한다', () => {
        const lines = ['첫 번째', '', '두 번째', '', '', '세 번째'];
        const blocks = buildParagraphs(lines, 2);
        expect(blocks[0].text).toContain('첫 번째');
        expect(blocks[0].text).toContain('두 번째');
        expect(blocks[1].text).toBe('세 번째');
    });
});

describe('trimEmptyParagraphs', () => {
    it('빈 텍스트 블록을 제거한다', () => {
        const blocks = [
            { type: 'narrative', text: '내용' },
            { type: 'narrative', text: '  ' },
            { type: 'narrative', text: '또 내용' },
        ];
        const result = trimEmptyParagraphs(blocks);
        expect(result).toHaveLength(2);
    });
});

describe('formatText 통합 테스트', () => {
    it('이중행간 파일: collapseDoubleSpacing 적용', () => {
        // 이중행간 파일 시뮬레이션 (10줄 + 행간 빈줄)
        const lines = [];
        for (let i = 0; i < 10; i++) {
            lines.push(`내용 줄 ${i}`);
            lines.push('');
        }
        // 단락 구분 (빈줄 2개+)
        lines.push('');
        lines.push('');
        for (let i = 0; i < 5; i++) {
            lines.push(`두번째 단락 ${i}`);
            lines.push('');
        }
        const blocks = formatText(lines);
        // 이중행간 축소 후 단락 분리됨
        expect(blocks.length).toBeGreaterThanOrEqual(2);
        expect(blocks[0].text).toContain('내용 줄');
    });

    it('계속/끝 마커를 제거한다', () => {
        const lines = ['내용', '---계속---', '다음', '---끝---', '마지막'];
        const blocks = formatText(lines);
        const allText = blocks.map(b => b.text).join(' ');
        expect(allText).not.toContain('계속');
        expect(allText).not.toContain('끝');
    });

    it('들여쓰기와 후행공백을 제거한다', () => {
        const lines = ['     들여쓴 내용     ', '  또 다른 줄  '];
        const blocks = formatText(lines);
        expect(blocks[0].text).toContain('들여쓴 내용');
        expect(blocks[0].text).not.toMatch(/^\s/);
    });

    it('자연 포맷 파일: unwrap ON 해도 변화 없음', () => {
        // 자연 포맷 = 다양한 길이의 줄, 폭감지 불가
        const lines = ['짧은 줄', '좀 더 긴 줄입니다', '아주 매우 더욱 길고 긴 줄'];
        const blocksOff = formatText(lines, { unwrap: false });
        const blocksOn = formatText(lines, { unwrap: true });
        expect(blocksOn).toEqual(blocksOff);
    });

    it('대화, 구분선, 제목 감지', () => {
        const lines = [
            ...Array(10).fill('가'.repeat(36)),
            '',
            '서술 내용이 시작됩니다.',
            '',
            '"대화 내용입니다."',
            '',
            '***',
            '',
            '제2장',
            '',
            '다음 장의 내용입니다.',
        ];
        const blocks = formatText(lines);
        expect(blocks.some(b => b.type === 'dialogue')).toBe(true);
        expect(blocks.some(b => b.type === 'separator')).toBe(true);
        expect(blocks.some(b => b.type === 'header')).toBe(true);
    });

    it('빈 입력에 대해 빈 배열 반환', () => {
        expect(formatText([])).toEqual([]);
    });
});

describe('visualWidth - 전각 문자 범위', () => {
    it('CJK 호환 한자(U+F900~U+FAFF)는 폭 2', () => {
        expect(visualWidth('豈﫿')).toBe(4);
    });

    it('CJK 호환 형태(U+FE30~U+FE4F)는 폭 2', () => {
        expect(visualWidth('︰﹏')).toBe(4);
    });

    it('전각 형태(U+FF01~U+FF60)는 폭 2', () => {
        expect(visualWidth('！｠')).toBe(4);
    });

    it('전각 기호(U+FFE0~U+FFE6)는 폭 2', () => {
        expect(visualWidth('￠￦')).toBe(4);
    });

    it('CJK 확장 B(U+20000~U+2FA1F)는 폭 2', () => {
        expect(visualWidth('\u{20000}\u{2FA1F}')).toBe(4);
    });

    it('각 범위의 경계 바깥 문자는 폭 1', () => {
        // 0xFB00(0xFAFF 직후), 0xFE50(0xFE4F 직후), 0xFF61(0xFF60 직후),
        // 0xFFE7(0xFFE6 직후) 는 모두 전각 범위 밖이다.
        expect(visualWidth('ﬀ﹐｡￧')).toBe(4);
    });
});

describe('unwrapForceBreaks - 병합 중단 조건', () => {
    // 시각 폭 60짜리 긴 줄 10개로 wrapWidth=60 을 확정시킨다.
    // threshold = 52, upperBound = 68, wrapWidth - 4 = 56
    const wide = 'a'.repeat(60);
    const establish = Array(10).fill(wide);

    it('현재 줄이 threshold 미만이면 다음 줄과 합치지 않는다', () => {
        const lines = [...establish, '', 'short', 'tail'];
        const result = unwrapForceBreaks(lines);
        // 'short'(폭 5)는 threshold 52 미만이므로 'tail'과 병합되지 않는다
        expect(result).toContain('short');
        expect(result).toContain('tail');
    });

    it('문장종결 부호로 끝나면서 wrapWidth-4 미만이면 합치지 않는다', () => {
        const terminated = 'b'.repeat(52) + '.'; // 폭 53: threshold 이상, 56 미만
        const lines = [...establish, '', terminated, 'continuation'];
        const result = unwrapForceBreaks(lines);
        expect(result).toContain(terminated);
        expect(result).toContain('continuation');
    });

    it('종결 부호로 끝나도 wrapWidth-4 이상이면 계속 합친다', () => {
        const terminated = 'b'.repeat(58) + '.'; // 폭 59: 56 이상 → 병합 계속
        const lines = [...establish, '', terminated, 'continuation'];
        const result = unwrapForceBreaks(lines);
        expect(result).toContain(`${terminated} continuation`);
    });
});

describe('buildParagraphs - 대시 구분선 판정', () => {
    it('공백 섞인 대시 5자 이상이고 대시가 3개 이상이면 구분선', () => {
        const blocks = buildParagraphs(['본문', '', '-  --', '', '다음']);
        expect(blocks.some(b => b.type === 'separator' && b.text === '-  --')).toBe(true);
    });

    it('공백 섞인 대시 5자 이상이라도 대시가 3개 미만이면 구분선이 아니다', () => {
        const blocks = buildParagraphs(['본문', '', '-   -', '', '다음']);
        expect(blocks.some(b => b.type === 'separator')).toBe(false);
        expect(blocks.some(b => b.text === '-   -')).toBe(true);
    });
});
