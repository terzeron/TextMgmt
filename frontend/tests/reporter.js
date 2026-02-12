export default class SummaryReporter {
    #testFiles = 0;
    #passedFiles = 0;
    #failedFiles = 0;
    #totalTests = 0;
    #passedTests = 0;
    #failedTests = 0;

    onTestModuleEnd(module) {
        this.#testFiles++;
        const passed = module.state() === 'passed';
        if (passed) this.#passedFiles++;
        else this.#failedFiles++;

        let modulePassedTests = 0;
        let moduleFailedTests = 0;
        for (const test of module.children.allTests()) {
            this.#totalTests++;
            if (test.result()?.state === 'passed') {
                this.#passedTests++;
                modulePassedTests++;
            } else {
                this.#failedTests++;
                moduleFailedTests++;
            }
        }

        // 파일별 진행 상태 출력
        const icon = passed ? '\x1b[32m✓\x1b[0m' : '\x1b[31m✗\x1b[0m';
        const name = module.moduleId.replace(/^.*?tests\//, 'tests/');
        const count = moduleFailedTests > 0
            ? `\x1b[32m${modulePassedTests}\x1b[0m / \x1b[31m${moduleFailedTests} failed\x1b[0m`
            : `${modulePassedTests} tests`;
        process.stdout.write(` ${icon} ${name} (${count})\n`);
    }

    onTestRunEnd() {
        process.on('beforeExit', () => {
            const write = (msg) => process.stdout.write(msg + '\n');
            write('');
            if (this.#failedFiles > 0) {
                write(` Test Files  \x1b[31m${this.#failedFiles} failed\x1b[0m | \x1b[32m${this.#passedFiles} passed\x1b[0m (${this.#testFiles})`);
            } else {
                write(` Test Files  \x1b[32m${this.#passedFiles} passed\x1b[0m (${this.#testFiles})`);
            }
            if (this.#failedTests > 0) {
                write(`      Tests  \x1b[31m${this.#failedTests} failed\x1b[0m | \x1b[32m${this.#passedTests} passed\x1b[0m (${this.#totalTests})`);
            } else {
                write(`      Tests  \x1b[32m${this.#passedTests} passed\x1b[0m (${this.#totalTests})`);
            }
        });
    }
}
