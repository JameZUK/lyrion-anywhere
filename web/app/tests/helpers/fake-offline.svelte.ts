// The downloads engine as the main screens see it (Wave 3.5 item 14): reactive like the real
// OfflinePlayer, recording what it was asked to do. Its own behaviour has its own tests.
export class FakeOffline {
	queue = $state<unknown[]>([]);
	index = $state(0);
	playing = $state(false);
	position = $state(0);
	duration = $state(0);
	cover = $state('');
	error = $state('');
	user = 'alice';
	calls: string[] = [];
	get active() {
		return this.queue.length > 0;
	}
	get current() {
		return this.queue[this.index];
	}
	toggle() {
		this.calls.push('toggle');
		this.playing = !this.playing;
	}
	pause() {
		this.calls.push('pause');
		this.playing = false;
	}
	async next() {
		this.calls.push('next');
		this.index++;
	}
	async previous() {
		this.calls.push('previous');
	}
	async jump(i: number) {
		this.calls.push(`jump ${i}`);
		this.index = i;
	}
	seek(s: number) {
		this.calls.push(`seek ${s}`);
		this.position = s;
	}
	stop() {
		this.calls.push('stop');
		this.queue = [];
	}
	forget() {}
}
