// 실제 화면 동작을 검사하기 위한 DOM 확장을 불러옵니다.
import '@testing-library/jest-dom/vitest';
import {cleanup} from '@testing-library/react';
import {afterEach} from 'vitest';
afterEach(cleanup);
