import js from '@eslint/js';

export default [
	{
		ignores: ['dist/**', 'target/**', 'node_modules/**'],
	},
	js.configs.recommended,
	{
		files: ['assets/zola/search.js', 'web/**/*.js'],
		languageOptions: {
			ecmaVersion: 2024,
			sourceType: 'script',
			globals: {
				alert: 'readonly',
				confirm: 'readonly',
				document: 'readonly',
				fetch: 'readonly',
				localStorage: 'readonly',
				location: 'readonly',
				window: 'readonly',
			},
		},
		rules: {
			eqeqeq: ['error', 'always'],
			indent: ['error', 'tab'],
			'no-var': 'error',
			'prefer-const': 'error',
			quotes: ['error', 'single', { avoidEscape: true }],
			semi: ['error', 'always'],
		},
	},
	{
		files: ['assets/zola/stl-viewer.js', 'assets/zola/three-model-viewer.js'],
		languageOptions: {
			ecmaVersion: 2024,
			sourceType: 'module',
			globals: {
				console: 'readonly',
				document: 'readonly',
				requestAnimationFrame: 'readonly',
				window: 'readonly',
			},
		},
		rules: {
			eqeqeq: ['error', 'always'],
			indent: ['error', 'tab'],
			'no-var': 'error',
			'prefer-const': 'error',
			quotes: ['error', 'single', { avoidEscape: true }],
			semi: ['error', 'always'],
		},
	},
];
