/// <reference types="vitest/globals" />

/**
 * React 的 act 环境开关。
 *
 * 测试里必须为 true，否则 `act(...)` 会报「当前测试环境不支持 act」。
 * @see src/test/setup.ts
 */
declare var IS_REACT_ACT_ENVIRONMENT: boolean;
