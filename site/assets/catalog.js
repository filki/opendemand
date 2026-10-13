(() => {
	const ua = navigator.userAgent || "";
	let os = null;
	if (/Windows/i.test(ua)) {
		os = "windows";
	} else if (/Macintosh|Mac OS X/i.test(ua) && !/iPhone|iPad|iPod/i.test(ua)) {
		os = "macos";
	} else if (/Linux/i.test(ua) && !/Android/i.test(ua)) {
		os = "linux";
	}
	if (os) {
		document.documentElement.dataset.os = os;
	}
})();
