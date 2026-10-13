// Bumped whenever this app changes a player's alarms, so Rooms (which shows each player's next
// alarm) reads them again. LMS sends no event for alarm changes.
class AlarmChanges {
	version = $state(0);
	bump() {
		this.version++;
	}
}

export const alarmChanges = new AlarmChanges();
