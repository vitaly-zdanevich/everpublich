(function () {
	const input = document.getElementById('site-search');
	const list = document.getElementById('search-results');
	if (!input || !list || !window.searchIndex) {
		return;
	}
	if (input.form) {
		input.form.addEventListener('submit', function (event) {
			event.preventDefault();
		});
	}
	function normalizeText(value) {
		const decoder = document.createElement('textarea');
		decoder.innerHTML = value || '';
		return decoder.value.replace(/\s+/g, ' ').trim();
	}
	function truncateText(value) {
		return value.length > 1000 ? value.slice(0, 997) + '...' : value;
	}
	function resultTitle(page, url) {
		const metadata = (window.everpublichSearchMetadata && window.everpublichSearchMetadata[url]) || {};
		const parts = [];
		const body = normalizeText(page.body);
		if (metadata.date) {
			parts.push(metadata.date);
		}
		if (body) {
			parts.push(body);
		}
		return truncateText(parts.join('\n\n') || page.title || url);
	}
	input.addEventListener('input', function () {
		const query = input.value.trim().toLowerCase();
		list.innerHTML = '';
		if (!query) {
			list.hidden = true;
			return;
		}
		const pages = (window.searchIndex.documentStore && window.searchIndex.documentStore.docs) || window.searchIndex.documents || {};
		Object.keys(pages).some(function (key) {
			const page = pages[key];
			const haystack = ((page.title || '') + ' ' + (page.body || '')).toLowerCase();
			if (haystack.indexOf(query) === -1) {
				return false;
			}
			const url = page.url || page.id || key;
			const item = document.createElement('li');
			const link = document.createElement('a');
			link.href = url;
			link.textContent = page.title || url;
			link.title = resultTitle(page, url);
			item.appendChild(link);
			list.appendChild(item);
			return list.children.length >= 10;
		});
		list.hidden = list.children.length === 0;
	});
})();
