(function () {
	const loaderPaths = {
		'3dm': 'three/addons/loaders/3DMLoader.js',
		'3mf': 'three/addons/loaders/3MFLoader.js',
		dae: 'three/addons/loaders/ColladaLoader.js',
		fbx: 'three/addons/loaders/FBXLoader.js',
		g: 'three/addons/loaders/GCodeLoader.js',
		gco: 'three/addons/loaders/GCodeLoader.js',
		gcode: 'three/addons/loaders/GCodeLoader.js',
		obj: 'three/addons/loaders/OBJLoader.js',
		ply: 'three/addons/loaders/PLYLoader.js',
		vox: 'three/addons/loaders/VOXLoader.js',
		vtk: 'three/addons/loaders/VTKLoader.js',
		vtp: 'three/addons/loaders/VTKLoader.js',
		xyz: 'three/addons/loaders/XYZLoader.js',
	};

	const loaderNames = {
		'3dm': 'Rhino3dmLoader',
		'3mf': 'ThreeMFLoader',
		dae: 'ColladaLoader',
		fbx: 'FBXLoader',
		g: 'GCodeLoader',
		gco: 'GCodeLoader',
		gcode: 'GCodeLoader',
		obj: 'OBJLoader',
		ply: 'PLYLoader',
		vox: 'VOXLoader',
		vtk: 'VTKLoader',
		vtp: 'VTKLoader',
		xyz: 'XYZLoader',
	};

	function initWhenReady() {
		initModelViewers().catch(function (error) {
			console.error('Everpublich 3D viewer failed', error);
		});
	}

	async function initModelViewers() {
		const roots = Array.from(document.querySelectorAll('.three-model-viewer[data-src]:not([data-everpublich-ready])'));
		if (!roots.length) {
			return;
		}
		const THREE = await import('three');
		const controlsModule = await import('three/addons/controls/OrbitControls.js');
		for (const root of roots) {
			await initModelViewer(root, THREE, controlsModule.OrbitControls);
		}
	}

	async function initModelViewer(root, THREE, OrbitControls) {
		root.dataset.everpublichReady = 'true';
		const kind = root.dataset.kind;
		const modulePath = loaderPaths[kind];
		const loaderName = loaderNames[kind];
		const canvasHost = root.querySelector('.three-model-viewer__canvas');
		if (!modulePath || !loaderName || !canvasHost) {
			return;
		}
		const loaderModule = await import(modulePath);
		const loader = new loaderModule[loaderName]();
		configureLoader(kind, loader);
		const scene = new THREE.Scene();
		const camera = new THREE.PerspectiveCamera(45, 1, 0.1, 1000);
		const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
		renderer.setPixelRatio(window.devicePixelRatio || 1);
		canvasHost.appendChild(renderer.domElement);

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

		loader.load(root.dataset.src, function (loaded) {
			const object = normalizeLoadedObject(THREE, loaded, loaderModule, kind);
			scene.add(object);
			frameObject(THREE, camera, controls, object);
			resize();
			animate(renderer, scene, camera, controls);
		}, undefined, function () {
			canvasHost.textContent = 'Could not load 3D model.';
		});

		window.addEventListener('resize', resize);
		resize();
	}

	function configureLoader(kind, loader) {
		if (kind === '3dm' && typeof loader.setLibraryPath === 'function') {
			loader.setLibraryPath('https://cdn.jsdelivr.net/npm/rhino3dm@8.17.0/');
		}
	}

	function normalizeLoadedObject(THREE, loaded, loaderModule, kind) {
		if (kind === 'vox' && Array.isArray(loaded) && loaderModule.VOXMesh) {
			const group = new THREE.Group();
			loaded.forEach(function (chunk) {
				group.add(new loaderModule.VOXMesh(chunk));
			});
			return group;
		}
		if (loaded && loaded.scene) {
			return loaded.scene;
		}
		if (loaded && loaded.isObject3D) {
			return loaded;
		}
		if (loaded && loaded.isBufferGeometry) {
			loaded.computeVertexNormals();
			return new THREE.Mesh(loaded, defaultMaterial(THREE));
		}
		if (loaded && loaded.children) {
			return loaded;
		}
		return new THREE.Object3D();
	}

	function defaultMaterial(THREE) {
		return new THREE.MeshStandardMaterial({ color: 0x8fcf9d, roughness: 0.55, metalness: 0.05 });
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
