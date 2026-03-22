import { beforeAll, afterAll, vi } from 'vitest';

const originalConsoleLog = console.log;
const originalConsoleError = console.error;
const originalConsoleWarn = console.warn;
let originalGetComputedStyle;

beforeAll(() => {
    console.log = vi.fn();
    console.error = vi.fn();
    console.warn = vi.fn();
    if (typeof window !== 'undefined' && window.getComputedStyle) {
        originalGetComputedStyle = window.getComputedStyle;
        window.getComputedStyle = (element, pseudoElt) => {
            const style = originalGetComputedStyle(element, pseudoElt);
            return new Proxy(style, {
                get(target, prop) {
                    if (prop === 'transitionDuration' || prop === 'transitionDelay') {
                        const value = target[prop];
                        return value && value !== '' ? value : '0s';
                    }
                    if (prop === 'getPropertyValue') {
                        return (name) => {
                            const value = target.getPropertyValue(name);
                            if ((name === 'transition-duration' || name === 'transition-delay') && (!value || value === '')) {
                                return '0s';
                            }
                            return value;
                        };
                    }
                    const value = target[prop];
                    return typeof value === 'function' ? value.bind(target) : value;
                },
            });
        };
    }
});

afterAll(() => {
    console.log = originalConsoleLog;
    console.error = originalConsoleError;
    console.warn = originalConsoleWarn;
    if (typeof window !== 'undefined' && originalGetComputedStyle) {
        window.getComputedStyle = originalGetComputedStyle;
    }
});
