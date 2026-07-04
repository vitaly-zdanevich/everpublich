const connectForm = document.getElementById('connect-form');
const status = document.getElementById('connect-status');
const statusText = status.querySelector('[data-status-text]');

connectForm.addEventListener('submit', async function (event) {
	event.preventDefault();
	status.hidden = false;
	statusText.textContent = 'Connecting to Evernote...';

	const formData = new FormData(connectForm);
	const response = await fetch('/api/connect', {
		method: 'POST',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify({ site_name: formData.get('site_name') }),
	});
	const json = await response.json();
	if (!response.ok || json.error) {
		statusText.textContent = json.error || 'Connection failed';
		return;
	}
	localStorage.setItem('everpublich_admin_token', json.admin_token);
	localStorage.setItem('everpublich_user_id', json.user_id);
	statusText.innerHTML = 'Website queued: <a href=\'' + json.website_url + '\'>' + json.website_url + '</a>. Check after a few minutes while notes download and the site builds.';
});
