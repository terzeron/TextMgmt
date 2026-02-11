// @vitest-environment jsdom
import { describe, it, expect } from 'vitest';
import { quasiRandomHue, vanDerCorput, getRandomLightColor, getRandomMediumColor } from '../src/Common';

// ── quasiRandomHue ──

describe('quasiRandomHue', () => {
    it('결정적(deterministic) 출력을 반환한다', () => {
        expect(quasiRandomHue(0)).toBe(0);
        expect(quasiRandomHue(1)).toBeCloseTo(0.6180339887498949, 10);
        expect(quasiRandomHue(1)).toBe(quasiRandomHue(1));
    });

    it('반환값이 0~1 범위 내에 있다', () => {
        for (let i = 0; i < 1000; i++) {
            const val = quasiRandomHue(i);
            expect(val).toBeGreaterThanOrEqual(0);
            expect(val).toBeLessThan(1);
        }
    });

    it('연속 값이 균등하게 분포한다 (equidistribution)', () => {
        const buckets = new Array(10).fill(0);
        const N = 1000;
        for (let i = 0; i < N; i++) {
            const bucket = Math.floor(quasiRandomHue(i) * 10);
            buckets[bucket]++;
        }
        // 각 버킷에 최소 50개 이상 (기대값 100)
        for (const count of buckets) {
            expect(count).toBeGreaterThan(50);
        }
    });
});

// ── vanDerCorput ──

describe('vanDerCorput', () => {
    it('base 2에서 올바른 값을 반환한다', () => {
        expect(vanDerCorput(0)).toBe(0);
        expect(vanDerCorput(1)).toBe(0.5);
        expect(vanDerCorput(2)).toBe(0.25);
        expect(vanDerCorput(3)).toBe(0.75);
    });

    it('base 3에서 올바른 값을 반환한다', () => {
        expect(vanDerCorput(1, 3)).toBeCloseTo(1 / 3, 10);
        expect(vanDerCorput(2, 3)).toBeCloseTo(2 / 3, 10);
        expect(vanDerCorput(3, 3)).toBeCloseTo(1 / 9, 10);
    });

    it('반환값이 0~1 범위 내에 있다', () => {
        for (let i = 0; i < 500; i++) {
            const val = vanDerCorput(i);
            expect(val).toBeGreaterThanOrEqual(0);
            expect(val).toBeLessThan(1);
        }
    });
});

// ── getRandomLightColor ──

describe('getRandomLightColor', () => {
    it('HSL 형식의 문자열을 반환한다', () => {
        const color = getRandomLightColor('test');
        expect(color).toMatch(/^hsl\(\d+, 55%, 90%\)$/);
    });

    it('같은 입력에 대해 같은 색상을 반환한다', () => {
        expect(getRandomLightColor('myCategory')).toBe(getRandomLightColor('myCategory'));
    });

    it('다른 입력에 대해 다른 색상을 반환한다', () => {
        const colors = new Set();
        const prefixes = [
            '소설', '에세이', '시', '역사', '과학', '철학', '경제', '예술',
            '문학', '인문', '사회', '자연', '공학', '의학', '교육', '종교',
        ];
        for (const prefix of prefixes) {
            colors.add(getRandomLightColor(prefix));
        }
        // 16개 입력에 대해 최소 14개 이상 다른 색상
        expect(colors.size).toBeGreaterThanOrEqual(14);
    });

    it('다양한 키에서 Hue가 유효 범위(0~360)이다', () => {
        const keys = ['test', 'negative', '음수케이스', '123', '!@#$%'];
        for (const key of keys) {
            const color = getRandomLightColor(key);
            expect(color).toMatch(/^hsl\(\d+, 55%, 90%\)$/);
            const hue = parseInt(color.match(/\d+/)[0]);
            expect(hue).toBeGreaterThanOrEqual(0);
            expect(hue).toBeLessThanOrEqual(360);
        }
    });
});

// ── getRandomMediumColor ──

describe('getRandomMediumColor', () => {
    it('HSL 형식의 문자열을 반환한다', () => {
        const color = getRandomMediumColor('test');
        expect(color).toMatch(/^hsl\(\d+, 50%, 55%\)$/);
    });

    it('같은 입력에 대해 같은 색상을 반환한다', () => {
        expect(getRandomMediumColor('abc')).toBe(getRandomMediumColor('abc'));
    });
});
