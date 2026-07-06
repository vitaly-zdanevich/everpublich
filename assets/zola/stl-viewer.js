(function () {
	function initWhenReady() {
		initStlViewers().catch(function (error) {
			console.error('Everpublich STL viewer failed', error);
		});
	}

	async function initStlViewers() {
		const roots = Array.from(document.querySelectorAll('.stl-viewer[data-src]:not([data-everpublich-ready])'));
		if (!roots.length) {
			return;
		}
		const THREE = await import('three');
		const controlsModule = await import('three/addons/controls/OrbitControls.js');
		const loaderModule = await import('three/addons/loaders/STLLoader.js');
		roots.forEach(function (root) {
			initStlViewer(root, THREE, controlsModule.OrbitControls, loaderModule.STLLoader);
		});
	}

	function initStlViewer(root, THREE, OrbitControls, STLLoader) {
		root.dataset.everpublichReady = 'true';
		const canvasHost = root.querySelector('.stl-viewer__canvas');
		if (!canvasHost) {
			return;
		}
		const scene = new THREE.Scene();
		const camera = new THREE.PerspectiveCamera(45, 1, 0.1, 1000);
		const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
		renderer.setPixelRatio(window.devicePixelRatio || 1);
		canvasHost.appendChild(renderer.domElement);

		const material = new THREE.MeshStandardMaterial({ color: 0x8fcf9d, roughness: 0.55, metalness: 0.05 });
		const ambient = new THREE.HemisphereLight(0xffffff, 0x303030, 2.2);
		const key = new THREE.DirectionalLight(0xffffff, 2.4);
		key.position.set(3, 4, 5);
		scene.add(ambient, key);

		const controls = new OrbitControls(camera, renderer.domElement);
		controls.enableDamping = true;
		controls.dampingFactor = 0.08;

		function resize() {
			const rect = canvasHost.getBoundingClientRect();
			const width = Math.max(1, Math.floor(rect.width));
			const height = Math.max(1, Math.floor(rect.height));
			camera.aspect = width / height;
			camera.updateProjectionMatrix();
			renderer.setSize(width, height, false);
		}

		new STLLoader().load(root.dataset.src, function (geometry) {
			geometry.computeVertexNormals();
			geometry.center();
			const mesh = new THREE.Mesh(geometry, material);
			scene.add(mesh);
			frameObject(THREE, camera, controls, mesh);
			resize();
			animate(renderer, scene, camera, controls);
		}, undefined, function () {
			canvasHost.textContent = 'Could not load STL model.';
		});

		window.addEventListener('resize', resize);
		resize();
	}

	function frameObject(THREE, camera, controls, object) {
		const box = new THREE.Box3().setFromObject(object);
		const size = box.getSize(new THREE.Vector3());
		const center = box.getCenter(new THREE.Vector3());
		const maxSize = Math.max(size.x, size.y, size.z, 1);
		const distance = maxSize * 2.2;
		camera.position.set(center.x + distance, center.y + distance * 0.7, center.z + distance);
		camera.near = Math.max(distance / 100, 0.01);
		camera.far = distance * 100;
		camera.updateProjectionMatrix();
		controls.target.copy(center);
		controls.update();
	}

	function animate(renderer, scene, camera, controls) {
		function render() {
			controls.update();
			renderer.render(scene, camera);
			requestAnimationFrame(render);
		}
		render();
	}

	if (document.readyState === 'loading') {
		document.addEventListener('DOMContentLoaded', initWhenReady);
	} else {
		initWhenReady();
	}
})();
