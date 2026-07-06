#!/usr/bin/env node
import { readFile, writeFile } from 'node:fs/promises';
import { minify } from 'html-minifier-terser';

const files = process.argv.slice(2);

if (files.length === 0) {
	console.error('Usage: node scripts/minify-html.mjs FILE [FILE...]');
	process.exit(2);
}

const options = {
	collapseBooleanAttributes: true,
	collapseWhitespace: true,
	decodeEntities: true,
	minifyCSS: true,
	minifyJS: true,
	removeComments: true,
	removeRedundantAttributes: true,
	removeScriptTypeAttributes: true,
	removeStyleLinkTypeAttributes: true,
	sortAttributes: true,
	sortClassName: true,
};

for (const file of files) {
	const html = await readFile(file, 'utf8');
	const minified = await minify(html, options);
	await writeFile(file, minified, 'utf8');
}
