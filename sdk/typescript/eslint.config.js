import js from '@eslint/js';
import ts from 'typescript-eslint';

export default ts.config(
  js.configs.recommended,
  ts.configs.recommendedTypeChecked,
  {
    files: ['src/**/*.ts', 'tests/**/*.ts', 'tools/**/*.mjs', '*.js', '*.ts'],
    languageOptions: {parserOptions: {project: './tsconfig.test.json', tsconfigRootDir: import.meta.dirname}},
    linterOptions: {reportUnusedDisableDirectives: 'error'},
  },
);
