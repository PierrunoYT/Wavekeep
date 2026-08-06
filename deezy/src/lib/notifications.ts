import { isPermissionGranted, requestPermission, sendNotification } from '@tauri-apps/plugin-notification';

interface PendingNotification {
  title: string;
  body: string;
}

const MAX_PENDING_NOTIFICATIONS = 10;
const BATCH_NOTIFICATION_THRESHOLD = 2;

class NotificationManager {
  private permissionGranted: boolean | null = null;
  private notificationsEnabled = false;
  private enableGeneration = 0;
  private pendingNotifications: PendingNotification[] = [];

  async checkPermission(generation = this.enableGeneration): Promise<boolean> {
    if (!this.isEnabledGeneration(generation)) return false;

    try {
      let permissionGranted = await isPermissionGranted();

      if (!this.isEnabledGeneration(generation)) return false;

      if (!permissionGranted) {
        const permission = await requestPermission();
        permissionGranted = permission === 'granted';
      }

      if (!this.isEnabledGeneration(generation)) return false;

      this.permissionGranted = permissionGranted;

      if (permissionGranted && this.pendingNotifications.length > 0) {
        await this.flushPendingNotifications(generation);
      }

      return this.isEnabledGeneration(generation) && permissionGranted;
    } catch (err) {
      console.error('Failed to check notification permission:', err);
      return false;
    }
  }

  setEnabled(enabled: boolean): void {
    this.notificationsEnabled = enabled;

    if (!enabled) {
      this.enableGeneration += 1;
      this.permissionGranted = null;
      this.pendingNotifications = [];
    }
  }

  getEnabled(): boolean {
    return this.notificationsEnabled;
  }

  private isEnabledGeneration(generation: number): boolean {
    return this.notificationsEnabled && generation === this.enableGeneration;
  }

  private async flushPendingNotifications(generation: number): Promise<void> {
    if (!this.isEnabledGeneration(generation) || this.pendingNotifications.length === 0) return;

    try {
      if (this.pendingNotifications.length < BATCH_NOTIFICATION_THRESHOLD) {
        for (const notif of this.pendingNotifications) {
          await this.sendNotificationInternal(notif.title, notif.body, generation);
        }
      } else {
        const count = this.pendingNotifications.length;
        await this.sendNotificationInternal(
          'Downloads Complete',
          `${count} tracks have finished downloading`,
          generation
        );
      }
    } finally {
      if (generation === this.enableGeneration) {
        this.pendingNotifications = [];
      }
    }
  }

  private async sendNotificationInternal(
    title: string,
    body: string,
    generation: number
  ): Promise<void> {
    if (!this.isEnabledGeneration(generation)) return;

    try {
      await sendNotification({ title, body });
    } catch (err) {
      console.error('Failed to send notification:', err);
    }
  }

  private addToPending(title: string, body: string): void {
    if (this.pendingNotifications.length >= MAX_PENDING_NOTIFICATIONS) {
      this.pendingNotifications.shift();
    }
    this.pendingNotifications.push({ title, body });
  }

  private async ensurePermission(generation: number): Promise<boolean> {
    if (!this.isEnabledGeneration(generation)) return false;

    if (this.permissionGranted === null) {
      await this.checkPermission(generation);
    }

    return this.isEnabledGeneration(generation) && (this.permissionGranted ?? false);
  }

  private async sendOrQueue(title: string, body: string): Promise<void> {
    const generation = this.enableGeneration;

    if (await this.ensurePermission(generation)) {
      await this.sendNotificationInternal(title, body, generation);
    } else if (this.isEnabledGeneration(generation)) {
      this.addToPending(title, body);
    } else {
      return;
    }
  }

  async notifyDownloadComplete(title: string, artist: string): Promise<void> {
    if (!this.notificationsEnabled) return;

    const notificationTitle = 'Download Complete';
    const notificationBody = `${title} - ${artist}`;

    await this.sendOrQueue(notificationTitle, notificationBody);
  }

  async notifyDownloadError(title: string, artist: string, error: string): Promise<void> {
    if (!this.notificationsEnabled) return;

    const notificationTitle = 'Download Failed';
    const truncatedError = error.length > 100 ? `${error.substring(0, 100)}...` : error;
    const notificationBody = `${title} - ${artist}\n${truncatedError}`;

    await this.sendOrQueue(notificationTitle, notificationBody);
  }

  async notifyBatchComplete(count: number): Promise<void> {
    if (!this.notificationsEnabled || count <= 0) return;

    const notificationTitle = 'Downloads Complete';
    const plural = count > 1 ? 's' : '';
    const notificationBody = `${count} track${plural} finished downloading`;

    await this.sendOrQueue(notificationTitle, notificationBody);
  }

  clearPending(): void {
    this.pendingNotifications = [];
  }

  getPendingCount(): number {
    return this.pendingNotifications.length;
  }
}

export const notificationManager = new NotificationManager();
