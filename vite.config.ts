// 로컬 개발 서버와 프런트 테스트 환경을 설정합니다.
import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import {previewPlugin} from './scripts/preview-plugin';
export default defineConfig({plugins:[react(),previewPlugin(process.cwd())],clearScreen:false,server:{port:1437,strictPort:true,host:'127.0.0.1'},test:{environment:'jsdom',setupFiles:['./tests/ui/setup.ts'],include:['tests/ui/**/*.test.tsx','tests/ui/**/*.test.ts']}});
