import {defineConfig} from 'vite'
import react from '@vitejs/plugin-react';
import basicSsl from '@vitejs/plugin-basic-ssl'

// https://vitejs.dev/config/
export default defineConfig({
    server: {
        https: true,
        proxy: {
          '/search': {
            target: 'http://localhost:8000',
            changeOrigin: true,
            secure: false
          }
        }
    },
    plugins: [basicSsl(), react()],
    optimizeDeps: {
        include: ["pdfjs-dist"]
    },
    build: {
        rollupOptions: {
            output: {
                manualChunks: {
                    pdfjs: ["pdfjs-dist"]
                }
            }
        }
    },
    test: {
        include: ['tests/**/*.test.{js,jsx}'],
        setupFiles: ['tests/setup.js'],
        reporters: ['./tests/reporter.js'],
        coverage: {
            provider: 'v8',
            reporter: ['text'],
            include: ['src/**/*.{js,jsx}'],
        },
    },
})
