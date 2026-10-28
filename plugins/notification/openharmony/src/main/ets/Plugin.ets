import { notificationManager } from '@kit.NotificationKit';
import { reminderAgentManager } from '@kit.BackgroundTasksKit';
import { BusinessError } from '@kit.BasicServicesKit';
import { wantAgent } from '@kit.AbilityKit';
import { Plugin, Invoke } from '@tauri/app';

// ─── Types ───

interface NotificationData {
  id?: number;
  channelId?: string;
  title?: string;
  body?: string;
  schedule?: object;
  largeBody?: string;
  summary?: string;
  actionTypeId?: string;
  group?: string;
  groupSummary?: boolean;
  sound?: string;
  inboxLines?: string[];
  icon?: string;
  largeIcon?: string;
  iconColor?: string;
  attachments?: object[];
  extra?: Record<string, Object>;
  ongoing?: boolean;
  autoCancel?: boolean;
  silent?: boolean;
}

// ─── Action Types (mirrors Rust ActionType) ───

interface ActionDef {
  id: string;
  title: string;
  requiresAuthentication: boolean;
  foreground: boolean;
  destructive: boolean;
  input: boolean;
  inputButtonTitle?: string;
  inputPlaceholder?: string;
}

interface RegisteredActionType {
  id: string;
  actions: ActionDef[];
  hiddenPreviewsBodyPlaceholder?: string;
  customDismissAction: boolean;
  allowInCarPlay: boolean;
  hiddenPreviewsShowTitle: boolean;
  hiddenPreviewsShowSubtitle: boolean;
}

interface RegisterActionTypesArgs {
  types: RegisteredActionType[];
}

// ─── Schedule variants (externally-tagged serde enum) ───

interface ScheduleAt {
  date: string;
  repeating: boolean;
  allowWhileIdle: boolean;
}

interface ScheduleIntervalFields {
  year?: number;
  month?: number;
  day?: number;
  weekday?: number;
  hour?: number;
  minute?: number;
  second?: number;
}

interface ScheduleInterval {
  interval: ScheduleIntervalFields;
  allowWhileIdle: boolean;
}

interface ScheduleEvery {
  interval: string;
  count: number;
  allowWhileIdle: boolean;
}

interface ScheduleWrapper {
  at?: ScheduleAt;
  interval?: ScheduleInterval;
  every?: ScheduleEvery;
}

// ─── Pending notification tracking ───

interface PendingEntry {
  id: number;
  title: string;
  body: string;
  schedule: object;
  // reminderAgentManager entries: the system reminder id.
  reminderId?: number;
}

interface PendingNotification {
  id: number;
  title: string;
  body: string;
  // Optional: entries recovered from the system after a process restart may
  // lose their schedule (one-shot countdown reminders store no absolute date).
  schedule?: object;
}

// Wire shape of the guest ActiveNotification (models.rs `ActiveNotification`,
// camelCase serde): id is required by the Rust model (no serde default — a
// missing id fails the whole Vec deserialization, hence the `?? 0` fallback
// at the call site); every other field is optional and defaults on the Rust
// side when absent (Option fields -> None, defaulted fields -> empty).
interface ActiveNotificationWire {
  id: number;
  tag?: string;
  title?: string;
  body?: string;
  group?: string;
  extra?: Record<string, Object>;
  sound?: string;
}

interface ChannelConfig {
  id: string;
  name: string;
  description?: string;
  importance?: number;
  sound?: string;
  lights?: boolean;
  lightColor?: string;
  vibration?: boolean;
  visibility?: number;
  slotType: notificationManager.SlotType;
}

interface ChannelPayload {
  id: string;
  name: string;
  description?: string;
  importance?: number;
  sound?: string;
  lights?: boolean;
  lightColor?: string;
  vibration?: boolean;
  visibility?: number;
}

interface NotificationIdRef {
  id: number;
}

interface BatchArgs {
  notifications: NotificationData[];
}

interface CancelArgs {
  notifications: number[];
}

interface RemoveActiveArgs {
  notifications: NotificationIdRef[];
}

interface DeleteChannelArgs {
  id: string;
}

// ─── Listener registration (register_listener / remove_listener) ───

// Wire format from JS addPluginListener: { event: string, handler: "__CHANNEL__:123" }
interface RegisterListenerArgs {
  event: string;
  handler: string;
}

// Wire format from JS PluginListener.unregister: { event: string, channelId: number }
interface RemoveListenerArgs {
  event: string;
  channelId: number;
}

// Payload emitted on the "actionPerformed" channel when a notification action
// button is clicked (via WantAgent cold-start or onNewWant warm-start).
interface ActionClickPayload {
  id: number;
  actionId: string;
}

// ─── Importance → SlotType mapping ───

function importanceToSlotType(importance: number | undefined): notificationManager.SlotType {
  switch (importance) {
    case 4: // Importance.High
      return notificationManager.SlotType.SOCIAL_COMMUNICATION;
    case 3: // Importance.Default
      return notificationManager.SlotType.SERVICE_INFORMATION;
    case 2: // Importance.Low
      return notificationManager.SlotType.CONTENT_INFORMATION;
    default: // Importance.Min(1), None(0), undefined
      return notificationManager.SlotType.OTHER_TYPES;
  }
}

// ─── Plugin Implementation ───

export class NotificationPlugin extends Plugin {
  private channelMap: Map<string, ChannelConfig> = new Map();
  // Track SlotType usage to prevent shared slots from being removed prematurely
  private slotTypeRefCount: Map<number, number> = new Map();
  // Registered action types: actionTypeId → definition with actions
  private actionTypeMap: Map<string, RegisteredActionType> = new Map();
  // Pending notifications scheduled as system reminders
  private pendingMap: Map<number, PendingEntry> = new Map();
  // Registered event listener channels: event name → channel id.
  // Populated by registerListener; used by onNotificationAction to emit.
  private listenerChannels: Map<string, number> = new Map();

  getCommands(): Map<string, (invoke: Invoke) => void> {
    const commands: Map<string, (invoke: Invoke) => void> = new Map();
    commands.set('show', (invoke: Invoke): void => { this.handleShow(invoke); });
    commands.set('batch', (invoke: Invoke): void => { this.handleBatch(invoke); });
    commands.set('cancel', (invoke: Invoke): void => { this.handleCancel(invoke); });
    commands.set('removeActive', (invoke: Invoke): void => { this.handleRemoveActive(invoke); });
    commands.set('getActive', (invoke: Invoke): void => { this.handleGetActive(invoke); });
    commands.set('getPending', (invoke: Invoke): void => { this.handleGetPending(invoke); });
    commands.set('requestPermissions', (invoke: Invoke): void => { this.handleRequestPermissions(invoke); });
    commands.set('checkPermissions', (invoke: Invoke): void => { this.handleCheckPermissions(invoke); });
    commands.set('createChannel', (invoke: Invoke): void => { this.handleCreateChannel(invoke); });
    commands.set('deleteChannel', (invoke: Invoke): void => { this.handleDeleteChannel(invoke); });
    commands.set('listChannels', (invoke: Invoke): void => { this.handleListChannels(invoke); });
    commands.set('registerActionTypes', (invoke: Invoke): void => { this.handleRegisterActionTypes(invoke); });
    commands.set('registerListener', (invoke: Invoke): void => { this.handleRegisterListener(invoke); });
    commands.set('removeListener', (invoke: Invoke): void => { this.handleRemoveListener(invoke); });
    return commands;
  }

  // ─── show ───

  private handleShow(invoke: Invoke): void {
    this.publishNotification(invoke);
  }

  private async publishNotification(invoke: Invoke): Promise<void> {
    try {
      const args: NotificationData = JSON.parse(invoke.parseArgs()) as NotificationData;
      const id = args.id ?? 0;

      // If a schedule is present, defer to the scheduling path (system
      // reminderAgentManager, #114)
      if (args.schedule != null) {
        this.scheduleNotification(id, args, invoke);
        return;
      }

      const request = await this.buildRequest(id, args);
      await notificationManager.publish(request);
      invoke.resolve(JSON.stringify(id));
    } catch (e) {
      const err = e as BusinessError;
      console.error('[NotificationPlugin] publish failed: ' + err.message);
      invoke.reject('Notification publish failed: ' + err.message);
    }
  }

  /**
   * Builds a NotificationRequest from NotificationData, including action buttons
   * if an actionTypeId is registered.
   */
  private async buildRequest(id: number, args: NotificationData): Promise<notificationManager.NotificationRequest> {
    const request: notificationManager.NotificationRequest = {
      id: id,
      content: this.buildContent(args),
      tapDismissed: args.autoCancel !== false
    };

    // channel_id → SlotType lookup
    if (args.channelId != null) {
      const channelConfig = this.channelMap.get(args.channelId);
      if (channelConfig != null) {
        request.notificationSlotType = channelConfig.slotType;
      } else {
        console.warn('[NotificationPlugin] channelId "' + args.channelId + '" not found in local mapping');
      }
    }

    // sound: must be rawfile name or sandbox URI
    if (args.sound != null) {
      request.sound = args.sound;
    }

    // ongoing: system "ongoing notification" flag
    // (NotificationRequest.isOngoing, @since 7)
    if (args.ongoing != null) {
      request.isOngoing = args.ongoing;
    }

    // group: system notification group (NotificationRequest.groupName, @since 8)
    if (args.group != null) {
      request.groupName = args.group;
    }

    // extra: stored on the system request (NotificationRequest.extraInfo,
    // @since 7) so getActive can round-trip it back to the guest (#209)
    if (args.extra != null) {
      request.extraInfo = args.extra as Record<string, Object>;
    }

    // action buttons: look up registered action type and build NotificationActionButtons
    if (args.actionTypeId != null) {
      const buttons = await this.buildActionButtons(args.actionTypeId, id);
      if (buttons != null && buttons.length > 0) {
        request.actionButtons = buttons;
      }
    }

    return request;
  }

  /**
   * Builds action buttons from a registered ActionType.
   * Each button gets a WantAgent that starts the app ability with action parameters.
   *
   * Action button clicks are dispatched via EntryAbility.handleNotificationAction
   * (cold-start in onCreate, warm-start in onNewWant), which calls
   * NotificationPlugin.onNotificationAction, which emits on the
   * "actionPerformed" listener channel registered via registerListener.
   */
  private async buildActionButtons(actionTypeId: string, notificationId: number): Promise<notificationManager.NotificationActionButton[] | null> {
    const actionType = this.actionTypeMap.get(actionTypeId);
    if (actionType == null) {
      console.warn('[NotificationPlugin] actionTypeId "' + actionTypeId + '" not registered');
      return null;
    }
    if (this.context == null) {
      console.warn('[NotificationPlugin] Cannot build action buttons: context is null');
      return null;
    }

    const bundleName = this.context.abilityInfo.bundleName;
    const abilityName = this.context.abilityInfo.name;

    const buttons: notificationManager.NotificationActionButton[] = [];
    for (let i = 0; i < actionType.actions.length; i++) {
      const action = actionType.actions[i];
      const wantAgentInfo: wantAgent.WantAgentInfo = {
        wants: [{
          bundleName: bundleName,
          abilityName: abilityName,
          parameters: {
            _tauriNotificationId: String(notificationId),
            _tauriActionId: action.id,
            _tauriActionTypeId: actionTypeId
          }
        }],
        actionType: wantAgent.OperationType.START_ABILITY,
        requestCode: 0,
        actionFlags: [wantAgent.WantAgentFlags.UPDATE_PRESENT_FLAG]
      };
      const wa = await wantAgent.getWantAgent(wantAgentInfo);
      // NotificationActionButton = { title, wantAgent } — no icon field in the
      // SDK type (notificationActionButton.d.ts); the button renders text-only.
      const button: notificationManager.NotificationActionButton = {
        title: action.title,
        wantAgent: wa
      };
      buttons.push(button);
    }
    return buttons;
  }

  // ─── scheduling (system reminders) ───

  /**
   * Schedules a notification (issue Eulogizethesun/tauri#114).
   *
   * Path: reminderAgentManager.publishReminder — the system proxies the
   * reminder when the app is backgrounded or killed, which is what
   * alarm/calendar use cases need. Any publishReminder failure rejects the
   * invoke (matching iOS, where a failing center.add rejects). Schedule →
   * reminder mapping:
   *   at (one-shot, future)  → ReminderRequestTimer (countdown)
   *   at (one-shot, due now) → immediate foreground publish (no reminder;
   *                            shows up in active, not pending)
   *   at (repeating, future) → ReminderRequestCalendar repeating monthly on
   *                            that day/time (approximation; Android repeats
   *                            with the initial delay as period — also an
   *                            approximation)
   *   at (repeating, past)   → reject
   *   interval               → ReminderRequestCalendar at the next field
   *                            match with repeat rules from the matched
   *                            fields (DateMatch semantics, matching
   *                            Android; year set → one-shot; month+day →
   *                            yearly, day → monthly, month alone → every
   *                            day of that month, weekday → weekly,
   *                            time-only → daily; no future match → reject)
   *   every (day/week/month/year, count=1) → Calendar anchored at now+1 unit
   *   every (hour/minute/second/twoWeeks, or count>1) → reject (Timer-type
   *                            reminders require a repeatInterval >= 86400 s
   *                            and API 26+, so sub-day repeats are not
   *                            expressible as system reminders)
   *
   * Interval rules mixing month/day with weekday (e.g. {month, weekday})
   * reject: the official docs do not define ReminderRequestCalendar's
   * combination semantics for daysOfWeek together with
   * repeatMonths/repeatDays (review R40 — revisit if device verification
   * defines them).
   *
   * Reminder datetime conventions (official API reference, do not mix up):
   *   LocalDateTime.month / repeatMonths: [1, 12]; repeatDays: [1, 31];
   *   daysOfWeek: Monday=1 .. Sunday=7 (tauri's interval.weekday is
   *   1=Sunday..7=Saturday — converted below).
   */
  private scheduleNotification(id: number, args: NotificationData, invoke: Invoke): void {
    try {
      const schedule = args.schedule as ScheduleWrapper;
      const reminder = this.buildReminderRequest(id, args, schedule);
      if (reminder != null) {
        this.publishReminder(id, args, reminder, invoke);
        return;
      }
      // null → one-shot 'at' already due: publish immediately in the
      // foreground (no system reminder; the notification shows up in
      // active, not pending).
      this.doScheduledPublish(id, args);
      invoke.resolve(JSON.stringify(id));
    } catch (e) {
      const err = e as BusinessError;
      console.error('[NotificationPlugin] schedule failed: ' + err.message);
      invoke.reject(err.message);
    }
  }

  /** Publishes a system reminder; any failure rejects the invoke (iOS-style). */
  private async publishReminder(
    id: number,
    args: NotificationData,
    reminder: reminderAgentManager.ReminderRequest,
    invoke: Invoke
  ): Promise<void> {
    try {
      const reminderId = await reminderAgentManager.publishReminder(reminder);
      this.pendingMap.set(id, {
        id: id,
        title: args.title ?? '',
        body: args.body ?? '',
        schedule: args.schedule as object,
        reminderId: reminderId
      });
      console.info('[NotificationPlugin] scheduled via reminderAgentManager: notificationId=' + id + ' reminderId=' + reminderId);
      invoke.resolve(JSON.stringify(id));
    } catch (e) {
      const err = e as BusinessError;
      // 201 (PUBLISH_AGENT_REMINDER permission missing) / 1700001
      // (notifications disabled) / 1700002 (reminder quota exceeded).
      // Attributions follow the official error-code reference.
      console.error('[NotificationPlugin] publishReminder failed (code=' + err.code + '): ' + err.message);
      let hint: string = '';
      if (err.code === 1700001) {
        hint = ' — notifications are not enabled for this app (requestEnableNotification or system settings)';
      } else if (err.code === 1700002) {
        hint = ' — reminder quota exceeded (a normal app without the AGC agent-reminder entitlement counts as quota 0, or the per-app reminder limit was reached)';
      } else if (err.code === 201) {
        hint = ' — ohos.permission.PUBLISH_AGENT_REMINDER is not granted';
      }
      invoke.reject('Schedule failed: publishReminder error ' + err.code + ': ' + err.message + hint);
    }
  }

  /**
   * Builds a ReminderRequest from a schedule. Returns null only for a
   * one-shot 'at' that is already due (the caller then publishes
   * immediately); every other invalid or non-expressible schedule throws.
   */
  private buildReminderRequest(
    id: number,
    args: NotificationData,
    schedule: ScheduleWrapper
  ): reminderAgentManager.ReminderRequest | null {
    const title = args.title ?? '';
    const body = args.body ?? '';
    const slotType = this.slotTypeFor(args);

    if (schedule.at != null) {
      const timestamp = Date.parse(schedule.at.date);
      if (isNaN(timestamp)) {
        throw new Error('Invalid schedule date: ' + schedule.at.date);
      }
      if (!schedule.at.repeating) {
        const seconds = Math.ceil((timestamp - Date.now()) / 1000);
        if (seconds < 1) {
          return null; // due now/overdue → the caller publishes immediately
        }
        const timer: reminderAgentManager.ReminderRequestTimer = {
          reminderType: reminderAgentManager.ReminderType.REMINDER_TYPE_TIMER,
          triggerTimeInSeconds: seconds,
          title: title,
          content: body,
          notificationId: id,
          slotType: slotType
        };
        return timer;
      }
      if (timestamp <= Date.now()) {
        // The legacy fallback clamped this to a 0-delay setInterval (a
        // notification firestorm); iOS rejects past repeating dates too.
        throw new Error('Scheduled date is in the past: ' + schedule.at.date);
      }
      const dt = new Date(timestamp);
      const calendar: reminderAgentManager.ReminderRequestCalendar = {
        reminderType: reminderAgentManager.ReminderType.REMINDER_TYPE_CALENDAR,
        dateTime: this.toLocalDateTime(dt),
        repeatMonths: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        repeatDays: [dt.getDate()],
        title: title,
        content: body,
        notificationId: id,
        slotType: slotType
      };
      return calendar;
    }

    if (schedule.interval != null) {
      const fields = schedule.interval.interval;
      const next = this.nextDateMatch(fields);
      if (next == null) {
        throw new Error('No future date matches the schedule fields');
      }
      const calendar: reminderAgentManager.ReminderRequestCalendar = {
        reminderType: reminderAgentManager.ReminderType.REMINDER_TYPE_CALENDAR,
        dateTime: this.toLocalDateTime(next),
        title: title,
        content: body,
        notificationId: id,
        slotType: slotType
      };
      if (fields.year == null) {
        // ReminderRequestCalendar's combination semantics for daysOfWeek
        // mixed with repeatMonths/repeatDays are not defined by the official
        // docs — those combinations reject instead of silently dropping
        // fields (review R40).
        if (fields.month != null && fields.day != null) {
          if (fields.weekday != null) {
            throw new Error('Unsupported schedule: combining month/day with weekday has no defined HarmonyOS reminder semantics');
          }
          // yearly on that month/day
          calendar.repeatMonths = [fields.month];
          calendar.repeatDays = [fields.day];
        } else if (fields.day != null) {
          if (fields.weekday != null) {
            throw new Error('Unsupported schedule: combining day with weekday has no defined HarmonyOS reminder semantics');
          }
          // monthly on that day of month (repeatMonths required with repeatDays)
          calendar.repeatMonths = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
          calendar.repeatDays = [fields.day];
        } else if (fields.weekday != null) {
          if (fields.month != null) {
            throw new Error('Unsupported schedule: combining month with weekday has no defined HarmonyOS reminder semantics');
          }
          // weekly on that weekday (tauri 1=Sun..7=Sat → OHOS Mon=1..Sun=7)
          calendar.daysOfWeek = [fields.weekday === 1 ? 7 : fields.weekday - 1];
        } else if (fields.month != null) {
          // month alone → every day of that month, each year (repeatMonths
          // paired with the full repeatDays range, mirroring the day-only
          // monthly mapping; months without days 29-31 just skip them)
          calendar.repeatMonths = [fields.month];
          calendar.repeatDays = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31];
        } else {
          // time-of-day only → daily
          calendar.daysOfWeek = [1, 2, 3, 4, 5, 6, 7];
        }
      }
      // year set → one-shot at that specific date
      return calendar;
    }

    if (schedule.every != null) {
      const unit = schedule.every.interval;
      const count = schedule.every.count > 0 ? schedule.every.count : 1;
      // The reminder system only supports calendar-rule repeats; arbitrary
      // N-unit periods and sub-day units have no equivalent (Timer-type
      // reminders require a repeatInterval >= 86400 s, API 26+).
      if (count !== 1 || unit === 'hour' || unit === 'minute' || unit === 'second' || unit === 'twoWeeks') {
        throw new Error("Unsupported schedule: every '" + unit + "' (count=" + count + ") has no HarmonyOS system-reminder equivalent");
      }
      const anchor = new Date();
      // Capture launch-time fields BEFORE any Date arithmetic: setMonth /
      // setFullYear overflow rolls short-month days into the next month
      // (Jan 31 + 1 month -> Mar 3) and would poison the repeat fields
      // below (review R39).
      const launchMonth = anchor.getMonth(); // 0-based
      const launchDay = anchor.getDate();
      let daysOfWeek: number[] | undefined = undefined;
      let repeatMonths: number[] | undefined = undefined;
      let repeatDays: number[] | undefined = undefined;
      switch (unit) {
        case 'day':
          anchor.setDate(anchor.getDate() + 1);
          daysOfWeek = [1, 2, 3, 4, 5, 6, 7];
          break;
        case 'week':
          anchor.setDate(anchor.getDate() + 7);
          // JS getDay 0=Sun..6=Sat → OHOS Mon=1..Sun=7
          daysOfWeek = [anchor.getDay() === 0 ? 7 : anchor.getDay()];
          break;
        case 'month':
          // Repeat on the launch day-of-month (months without that day are
          // expected to be skipped by the system, matching the repeating
          // `at` semantics).
          repeatMonths = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
          repeatDays = [launchDay];
          // First trigger: next month, day clamped into that month's length.
          // setDate(1) FIRST — clamping after the rollover pins the wrong
          // month (Jan 31 -> Mar 3 -> setDate(28) yields Mar 28, not Feb 28).
          anchor.setDate(1);
          anchor.setMonth(anchor.getMonth() + 1);
          anchor.setDate(Math.min(launchDay, this.daysInMonth(anchor.getFullYear(), anchor.getMonth())));
          break;
        case 'year':
          // Repeat on the launch month/day (a Feb-29 schedule is expected
          // to fire only in leap years).
          repeatMonths = [launchMonth + 1];
          repeatDays = [launchDay];
          anchor.setDate(1);
          anchor.setFullYear(anchor.getFullYear() + 1);
          anchor.setDate(Math.min(launchDay, this.daysInMonth(anchor.getFullYear(), anchor.getMonth())));
          break;
        default:
          throw new Error("Unsupported schedule: every '" + unit + "' has no HarmonyOS system-reminder equivalent");
      }
      const calendar: reminderAgentManager.ReminderRequestCalendar = {
        reminderType: reminderAgentManager.ReminderType.REMINDER_TYPE_CALENDAR,
        dateTime: this.toLocalDateTime(anchor),
        title: title,
        content: body,
        notificationId: id,
        slotType: slotType,
        daysOfWeek: daysOfWeek,
        repeatMonths: repeatMonths,
        repeatDays: repeatDays
      };
      return calendar;
    }

    throw new Error('Unsupported schedule format');
  }

  /** Resolves the notification SlotType for a payload (channel map → importance default). */
  private slotTypeFor(args: NotificationData): notificationManager.SlotType {
    if (args.channelId != null) {
      const channelConfig = this.channelMap.get(args.channelId);
      if (channelConfig != null) {
        return channelConfig.slotType;
      }
    }
    return importanceToSlotType(undefined);
  }

  /** Converts a Date to a reminder LocalDateTime (months are 1-based, unlike JS Date). */
  private toLocalDateTime(dt: Date): reminderAgentManager.LocalDateTime {
    return {
      year: dt.getFullYear(),
      month: dt.getMonth() + 1,
      day: dt.getDate(),
      hour: dt.getHours(),
      minute: dt.getMinutes(),
      second: dt.getSeconds()
    };
  }

  /** Last day of a JS 0-based month, e.g. daysInMonth(2027, 1) === 28. */
  private daysInMonth(year: number, month: number): number {
    return new Date(year, month + 1, 0).getDate();
  }

  /**
   * Finds the next time matching the set DateMatch fields (Android DateMatch
   * semantics: every set field must equal the value, unset fields are
   * wildcards). Day-by-day forward search bounded to ~4 years (covers Feb-29
   * anchors); returns null when nothing matches in that window.
   */
  private nextDateMatch(fields: ScheduleIntervalFields): Date | null {
    const anySet = fields.year != null || fields.month != null || fields.day != null
      || fields.weekday != null || fields.hour != null || fields.minute != null || fields.second != null;
    if (!anySet) {
      return null;
    }
    // tauri weekday is 1=Sunday..7=Saturday; JS Date.getDay() is 0=Sunday..6=Saturday
    const wantedWeekday = fields.weekday != null ? fields.weekday - 1 : -1;
    const start = new Date();
    for (let dayOffset = 0; dayOffset <= 1460; dayOffset++) {
      const candidate = new Date(
        start.getFullYear(), start.getMonth(), start.getDate() + dayOffset,
        fields.hour ?? 0, fields.minute ?? 0, fields.second ?? 0, 0
      );
      if (fields.year != null && candidate.getFullYear() !== fields.year) {
        continue;
      }
      if (fields.month != null && candidate.getMonth() + 1 !== fields.month) {
        continue;
      }
      if (fields.day != null && candidate.getDate() !== fields.day) {
        continue;
      }
      if (wantedWeekday >= 0 && candidate.getDay() !== wantedWeekday) {
        continue;
      }
      if (candidate.getTime() <= Date.now()) {
        continue;
      }
      return candidate;
    }
    return null;
  }

  /**
   * Immediate foreground publish for one-shot `at` schedules that are
   * already due (review R40: no system reminder, no timer fallback — the
   * notification shows up in active, not pending). Best-effort: publish
   * failures are logged, not rejected.
   */
  private async doScheduledPublish(id: number, args: NotificationData): Promise<void> {
    try {
      const request = await this.buildRequest(id, args);
      await notificationManager.publish(request);
    } catch (e) {
      const err = e as BusinessError;
      console.error('[NotificationPlugin] scheduled publish failed: ' + err.message);
    }
  }

  private buildContent(args: NotificationData): notificationManager.NotificationContent {
    const title = args.title ?? '';
    const text = args.body ?? '';
    const additionalText = args.summary ?? '';

    // long text style when largeBody is present
    if (args.largeBody != null && args.largeBody.length > 0) {
      const longContent: notificationManager.NotificationLongTextContent = {
        title: title,
        text: text,
        additionalText: additionalText,
        longText: args.largeBody,
        briefText: text,
        expandedTitle: title
      };
      const content: notificationManager.NotificationContent = {
        notificationContentType: notificationManager.ContentType.NOTIFICATION_CONTENT_LONG_TEXT,
        longText: longContent
      };
      return content;
    }

    // multiline style when inboxLines present
    if (args.inboxLines != null && args.inboxLines.length > 0) {
      const multiContent: notificationManager.NotificationMultiLineContent = {
        title: title,
        text: text,
        additionalText: additionalText,
        lines: args.inboxLines,
        briefText: args.summary ?? '',
        longTitle: title
      };
      const content: notificationManager.NotificationContent = {
        notificationContentType: notificationManager.ContentType.NOTIFICATION_CONTENT_MULTILINE,
        multiLine: multiContent
      };
      return content;
    }

    // default: basic text
    const basicContent: notificationManager.NotificationBasicContent = {
      title: title,
      text: text,
      additionalText: additionalText
    };
    const content: notificationManager.NotificationContent = {
      notificationContentType: notificationManager.ContentType.NOTIFICATION_CONTENT_BASIC_TEXT,
      normal: basicContent
    };
    return content;
  }

  // ─── batch ───

  private handleBatch(invoke: Invoke): void {
    this.publishBatch(invoke);
  }

  private async publishBatch(invoke: Invoke): Promise<void> {
    try {
      const args: BatchArgs = JSON.parse(invoke.parseArgs()) as BatchArgs;
      const notifications: NotificationData[] = Array.isArray(args) ? args as NotificationData[] : args.notifications;
      const ids: number[] = [];

      // Start auto-IDs at 10000 to avoid conflicts with user-defined IDs (typically small numbers)
      let nextAutoId: number = 10000;

      for (const data of notifications) {
        const id = data.id ?? nextAutoId++;
        const request = await this.buildRequest(id, data);
        await notificationManager.publish(request);
        ids.push(id);
      }

      invoke.resolve(JSON.stringify(ids));
    } catch (e) {
      const err = e as BusinessError;
      console.error('[NotificationPlugin] batch failed: ' + err.message);
      invoke.reject('Batch publish failed: ' + err.message);
    }
  }

  // ─── cancel ───

  private handleCancel(invoke: Invoke): void {
    this.cancelNotifications(invoke);
  }

  private async cancelNotifications(invoke: Invoke): Promise<void> {
    try {
      const argsStr = invoke.parseArgs();

      if (argsStr === '' || argsStr === 'null' || argsStr === '{}') {
        // cancelAll: clear all pending reminders + system notifications
        await this.clearAllPending();
        try {
          await reminderAgentManager.cancelAllReminders();
        } catch (reminderErr) {
          // Tolerated: nothing published, or the reminder service is
          // unavailable — notification cancelAll below still runs.
          console.warn('[NotificationPlugin] cancelAllReminders failed: ' + (reminderErr as BusinessError).message);
        }
        await notificationManager.cancelAll();
        invoke.resolve('');
        return;
      }

      const args: CancelArgs = JSON.parse(argsStr) as CancelArgs;
      const notifications: number[] = args.notifications;

      if (notifications != null && notifications.length > 0) {
        for (const id of notifications) {
          await this.clearPending(id);
          try {
            await notificationManager.cancel(id);
          } catch (cancelErr) {
            // Silently ignore if notification doesn't exist (OHOS throws, Android doesn't)
            console.warn('[NotificationPlugin] cancel id=' + id + ' failed (may not exist): ' + (cancelErr as BusinessError).message);
          }
        }
      } else {
        await this.clearAllPending();
        await notificationManager.cancelAll();
      }

      invoke.resolve('');
    } catch (e) {
      const err = e as BusinessError;
      console.error('[NotificationPlugin] cancel failed: ' + err.message);
      invoke.reject('Cancel failed: ' + err.message);
    }
  }

  /**
   * Clears a single pending entry (system reminder) by notification ID.
   * When the entry is no longer tracked (e.g. after a process restart the
   * in-memory map is gone while the system-proxied reminder is still
   * valid), reconciles with the system and cancels every valid reminder
   * published with this notification ID (#123). Best-effort: failures are
   * logged and swallowed.
   */
  private async clearPending(id: number): Promise<void> {
    const entry = this.pendingMap.get(id);
    if (entry != null) {
      if (entry.reminderId != null) {
        const reminderId = entry.reminderId;
        try {
          // 1700003 (already fired/removed) is tolerated.
          await reminderAgentManager.cancelReminder(reminderId);
        } catch (e) {
          console.warn('[NotificationPlugin] cancelReminder(' + reminderId + ') failed: ' + (e as BusinessError).message);
        }
      }
      this.pendingMap.delete(id);
      return;
    }
    // Map miss — the reminder id mapping is lost across process restarts, but
    // every reminder published by this plugin carries the application
    // notification id in reminderReq.notificationId (#123). Reminders whose
    // notificationId is 0/undefined were not published by this plugin (the
    // field defaults to 0) — never cancel those.
    try {
      const reminders: reminderAgentManager.ReminderInfo[] = await reminderAgentManager.getAllValidReminders();
      for (const info of reminders) {
        const notificationId = info.reminderReq.notificationId;
        if (notificationId == null || notificationId === 0) {
          continue;
        }
        if (notificationId !== id) {
          continue;
        }
        try {
          // 1700003 (already fired/removed) is tolerated.
          await reminderAgentManager.cancelReminder(info.reminderId);
        } catch (e) {
          console.warn('[NotificationPlugin] cancelReminder(' + info.reminderId + ') failed: ' + (e as BusinessError).message);
        }
      }
    } catch (e) {
      console.warn('[NotificationPlugin] getAllValidReminders failed: ' + (e as BusinessError).message);
    }
  }

  /** Clears all pending reminder entries. */
  private async clearAllPending(): Promise<void> {
    const keys: number[] = [];
    for (const key of this.pendingMap.keys()) {
      keys.push(key);
    }
    for (let i = 0; i < keys.length; i++) {
      await this.clearPending(keys[i]);
    }
  }

  // ─── registerActionTypes ───

  private handleRegisterActionTypes(invoke: Invoke): void {
    this.registerActionTypes(invoke);
  }

  private async registerActionTypes(invoke: Invoke): Promise<void> {
    try {
      const args: RegisterActionTypesArgs = JSON.parse(invoke.parseArgs()) as RegisterActionTypesArgs;
      const types: RegisteredActionType[] = args.types;
      if (types != null) {
        for (let i = 0; i < types.length; i++) {
          const t = types[i];
          this.actionTypeMap.set(t.id, t);
        }
      }
      invoke.resolve('');
    } catch (e) {
      const err = e as BusinessError;
      console.error('[NotificationPlugin] registerActionTypes failed: ' + err.message);
      invoke.reject('registerActionTypes failed: ' + err.message);
    }
  }

  // ─── registerListener ───

  private handleRegisterListener(invoke: Invoke): void {
    try {
      const args: RegisterListenerArgs = JSON.parse(invoke.parseArgs()) as RegisterListenerArgs;
      const channelId = Plugin.parseChannelId(args.handler);
      if (channelId == null) {
        invoke.reject('registerListener: invalid channel handler: ' + args.handler);
        return;
      }
      this.listenerChannels.set(args.event, channelId);
      console.info('[NotificationPlugin] Listener registered for event: ' + args.event + ' (channel ' + channelId + ')');
      // Note: "actionPerformed" events originate from EntryAbility.handleNotificationAction
      // (WantAgent cold-start / onNewWant warm-start). "notification" events
      // (onNotificationReceived) have no corresponding OHOS subscription API;
      // registration succeeds but no events will be delivered.
      invoke.resolve('');
    } catch (e) {
      const err = e as BusinessError;
      console.error('[NotificationPlugin] registerListener failed: ' + err.message);
      invoke.reject('registerListener failed: ' + err.message);
    }
  }

  // ─── removeListener ───

  private handleRemoveListener(invoke: Invoke): void {
    try {
      const args: RemoveListenerArgs = JSON.parse(invoke.parseArgs()) as RemoveListenerArgs;
      this.listenerChannels.delete(args.event);
      console.info('[NotificationPlugin] Listener removed for event: ' + args.event);
      invoke.resolve('');
    } catch (e) {
      const err = e as BusinessError;
      console.error('[NotificationPlugin] removeListener failed: ' + err.message);
      invoke.reject('removeListener failed: ' + err.message);
    }
  }

  // ─── notification action dispatch (called by EntryAbility) ───

  /**
   * Called by EntryAbility.handleNotificationAction when a notification action
   * button is clicked (via WantAgent). Emits the action click payload on the
   * "actionPerformed" listener channel.
   */
  onNotificationAction(notificationId: number, actionId: string, actionTypeId: string): void {
    const channelId = this.listenerChannels.get('actionPerformed');
    if (channelId != null) {
      const payload: ActionClickPayload = {
        id: notificationId,
        actionId: actionId
      };
      this.emit(channelId, payload);
    } else {
      console.warn('[NotificationPlugin] No listener registered for actionPerformed');
    }
  }

  // ─── getPending ───

  private handleGetPending(invoke: Invoke): void {
    this.getPendingNotifications(invoke);
  }

  /**
   * Assembles the pending list with the system reminder agent as the source of
   * truth (#123): Map entries whose system reminder is gone are pruned, and
   * reminders that only exist in the system (e.g. after a process restart)
   * are recovered. When system enumeration fails, degrades to the historical
   * Map-only behavior.
   */
  private async getPendingNotifications(invoke: Invoke): Promise<void> {
    try {
      const pending: PendingNotification[] = [];

      let reminders: reminderAgentManager.ReminderInfo[] | null = null;
      try {
        reminders = await reminderAgentManager.getAllValidReminders();
      } catch (e) {
        console.warn('[NotificationPlugin] getAllValidReminders failed: ' + (e as BusinessError).message);
      }

      // Plugin-published reminders (notificationId set) keyed by application
      // notification id, plus the ids of every still-valid system reminder.
      // Reminders whose notificationId is 0/undefined were not published by
      // this plugin — their ids are only tracked for validity checks.
      const systemByNotificationId: Map<number, reminderAgentManager.ReminderInfo> = new Map();
      const validReminderIds: number[] = [];
      if (reminders != null) {
        for (const info of reminders) {
          validReminderIds.push(info.reminderId);
          const notificationId = info.reminderReq.notificationId;
          if (notificationId != null && notificationId !== 0 && !systemByNotificationId.has(notificationId)) {
            systemByNotificationId.set(notificationId, info);
          }
        }
      }

      // Prune Map entries whose system reminder already fired/was canceled.
      // (Map entries are reminder-only since the foreground-timer fallback
      // was removed, so every entry carries a reminderId.)
      if (reminders != null) {
        const staleKeys: number[] = [];
        for (const entry of this.pendingMap.values()) {
          if (entry.reminderId != null && !validReminderIds.includes(entry.reminderId)) {
            staleKeys.push(entry.id);
          }
        }
        for (let i = 0; i < staleKeys.length; i++) {
          this.pendingMap.delete(staleKeys[i]);
        }
      }

      // Surviving Map entries are output verbatim.
      for (const entry of this.pendingMap.values()) {
        pending.push({
          id: entry.id,
          title: entry.title,
          body: entry.body,
          schedule: entry.schedule
        });
      }

      // Recover entries that only exist in the system (process restart):
      // reminders published by this plugin whose notification id is no longer
      // tracked in the Map.
      if (reminders != null) {
        for (const notificationId of systemByNotificationId.keys()) {
          if (this.pendingMap.has(notificationId)) {
            continue;
          }
          const info = systemByNotificationId.get(notificationId);
          if (info == null) {
            continue;
          }
          const recovered: PendingNotification = {
            id: notificationId,
            title: info.reminderReq.title ?? '',
            body: info.reminderReq.content ?? ''
          };
          const schedule: ScheduleWrapper | null = this.scheduleFromReminder(info.reminderReq);
          if (schedule != null) {
            recovered.schedule = schedule;
          }
          pending.push(recovered);
        }
      }

      invoke.resolve(JSON.stringify(pending));
    } catch (e) {
      const err = e as BusinessError;
      console.error('[NotificationPlugin] getPending failed: ' + err.message);
      invoke.resolve(JSON.stringify([]));
    }
  }

  /**
   * Best-effort inverse of buildReminderRequest for entries recovered from the
   * system (#123). Returns null when no schedule can be rebuilt — countdown
   * reminders (ReminderRequestTimer) only store a relative trigger offset, so
   * their original absolute date is not recoverable and the caller omits the
   * schedule field.
   */
  private scheduleFromReminder(req: reminderAgentManager.ReminderRequest): ScheduleWrapper | null {
    if (req.reminderType !== reminderAgentManager.ReminderType.REMINDER_TYPE_CALENDAR) {
      return null;
    }
    const calendar = req as reminderAgentManager.ReminderRequestCalendar;
    const dt: reminderAgentManager.LocalDateTime = calendar.dateTime;
    // Mirror of toLocalDateTime: reconstruct in the local timezone, then emit
    // an ISO string with UTC offset (the Rust side parses time::OffsetDateTime).
    const date = new Date(dt.year, dt.month - 1, dt.day, dt.hour, dt.minute, dt.second ?? 0);
    const iso = date.toISOString();
    const daysOfWeek = calendar.daysOfWeek;
    const repeatMonths = calendar.repeatMonths;
    const repeatDays = calendar.repeatDays;
    if (daysOfWeek != null && daysOfWeek.length === 7) {
      // daily (forward: every 'day' / time-of-day-only interval)
      const every: ScheduleEvery = { interval: 'day', count: 1, allowWhileIdle: false };
      const wrapper: ScheduleWrapper = { every: every };
      return wrapper;
    }
    if (daysOfWeek != null && daysOfWeek.length === 1) {
      // weekly on that weekday (forward: every 'week' / weekday interval)
      const every: ScheduleEvery = { interval: 'week', count: 1, allowWhileIdle: false };
      const wrapper: ScheduleWrapper = { every: every };
      return wrapper;
    }
    if (repeatMonths != null && repeatMonths.length === 12 && repeatDays != null && repeatDays.length === 1) {
      // monthly on that day (forward: every 'month' / day interval / repeating at)
      const every: ScheduleEvery = { interval: 'month', count: 1, allowWhileIdle: false };
      const wrapper: ScheduleWrapper = { every: every };
      return wrapper;
    }
    if (repeatMonths != null && repeatMonths.length === 1 && repeatDays != null && repeatDays.length === 1) {
      // yearly on that month/day (forward: every 'year' / month+day interval)
      const every: ScheduleEvery = { interval: 'year', count: 1, allowWhileIdle: false };
      const wrapper: ScheduleWrapper = { every: every };
      return wrapper;
    }
    if (daysOfWeek == null && repeatMonths == null && repeatDays == null) {
      // one-shot at a specific date (forward: interval with year set)
      const at: ScheduleAt = { date: iso, repeating: false, allowWhileIdle: false };
      const wrapper: ScheduleWrapper = { at: at };
      return wrapper;
    }
    // Any other repeat shape has no exact guest-API equivalent — report it as
    // a repeating schedule anchored at the reminder datetime.
    const at: ScheduleAt = { date: iso, repeating: true, allowWhileIdle: false };
    const wrapper: ScheduleWrapper = { at: at };
    return wrapper;
  }

  // ─── getActive ───

  private handleGetActive(invoke: Invoke): void {
    this.getActiveNotifications(invoke);
  }

  /**
   * Lists the app's active (published, not yet removed) notifications from
   * notificationManager.getActiveNotifications (#209) — the system is the
   * source of truth, so the list survives app restarts.
   *
   * NotificationRequest → guest ActiveNotification mapping:
   *   id → id (request id, defaults to 0 — required by the Rust model)
   *   label → tag (never set by this plugin; normally absent)
   *   content.{normal|longText|multiLine}.{title,text} → title/body
   *   groupName → group
   *   sound → sound
   *   extraInfo → extra (round-trip of Options.extra via buildRequest)
   * Not recoverable: schedule (already delivered), actionTypeId,
   * attachments, groupSummary, data (kept empty) — no system equivalent.
   *
   * Error codes are 401/1600001/1600002/1600003 (service errors only), so a
   * failure degrades to [] like getPending instead of rejecting.
   */
  private async getActiveNotifications(invoke: Invoke): Promise<void> {
    try {
      const requests: notificationManager.NotificationRequest[] =
        await notificationManager.getActiveNotifications();
      const active: ActiveNotificationWire[] = [];
      for (const req of requests) {
        let title: string | undefined = undefined;
        let body: string | undefined = undefined;
        const content = req.content;
        if (content != null) {
          if (content.normal != null) {
            title = content.normal.title;
            body = content.normal.text;
          } else if (content.longText != null) {
            title = content.longText.title;
            body = content.longText.text;
          } else if (content.multiLine != null) {
            title = content.multiLine.title;
            body = content.multiLine.text;
          }
        }
        const wire: ActiveNotificationWire = {
          id: req.id ?? 0,
          title: title,
          body: body
        };
        if (req.label != null) {
          wire.tag = req.label;
        }
        if (req.groupName != null) {
          wire.group = req.groupName;
        }
        if (req.sound != null) {
          wire.sound = req.sound;
        }
        if (req.extraInfo != null) {
          wire.extra = req.extraInfo as Record<string, Object>;
        }
        active.push(wire);
      }
      invoke.resolve(JSON.stringify(active));
    } catch (e) {
      const err = e as BusinessError;
      console.error('[NotificationPlugin] getActive failed: ' + err.message);
      invoke.resolve(JSON.stringify([]));
    }
  }

  // ─── removeActive ───

  private handleRemoveActive(invoke: Invoke): void {
    this.removeActive(invoke);
  }

  private async removeActive(invoke: Invoke): Promise<void> {
    try {
      const argsStr = invoke.parseArgs();

      if (argsStr === '' || argsStr === 'null' || argsStr === '{}') {
        await notificationManager.cancelAll();
        invoke.resolve('');
        return;
      }

      const args: RemoveActiveArgs = JSON.parse(argsStr) as RemoveActiveArgs;
      const notifications: NotificationIdRef[] = args.notifications;

      if (notifications != null && notifications.length > 0) {
        for (const n of notifications) {
          try {
            await notificationManager.cancel(n.id);
          } catch (cancelErr) {
            // Silently ignore if notification doesn't exist
            console.warn('[NotificationPlugin] removeActive id=' + n.id + ' failed (may not exist): ' + (cancelErr as BusinessError).message);
          }
        }
      } else {
        await notificationManager.cancelAll();
      }

      invoke.resolve('');
    } catch (e) {
      const err = e as BusinessError;
      console.error('[NotificationPlugin] removeActive failed: ' + err.message);
      invoke.reject('Remove active failed: ' + err.message);
    }
  }

  // ─── requestPermissions ───

  private handleRequestPermissions(invoke: Invoke): void {
    this.requestPermissions(invoke);
  }

  private async requestPermissions(invoke: Invoke): Promise<void> {
    try {
      // Check current state first
      const isEnabled = await notificationManager.isNotificationEnabled();
      if (isEnabled) {
        invoke.resolve(JSON.stringify({ permissionState: 'granted' }));
        return;
      }

      // Request permission — requires UIAbilityContext from Plugin base class
      if (this.context != null) {
        await notificationManager.requestEnableNotification(this.context);
      } else {
        console.warn('[NotificationPlugin] UIAbilityContext not available. User must enable notifications in system settings.');
        invoke.resolve(JSON.stringify({ permissionState: 'denied' }));
        return;
      }
      // Re-check actual state after request (user may have denied)
      const nowEnabled = await notificationManager.isNotificationEnabled();
      const state = nowEnabled ? 'granted' : 'denied';
      invoke.resolve(JSON.stringify({ permissionState: state }));
    } catch (e) {
      const err = e as BusinessError;
      if (err.code === 1600004) {
        // User previously denied — cannot re-prompt
        console.warn('[NotificationPlugin] Notification permission previously denied. User must enable in system settings.');
        invoke.resolve(JSON.stringify({ permissionState: 'denied' }));
      } else {
        console.error('[NotificationPlugin] requestPermissions failed: code=' + err.code + ', message=' + err.message);
        invoke.reject('requestPermissions failed: code=' + err.code + ', message=' + err.message);
      }
    }
  }

  // ─── checkPermissions ───

  private handleCheckPermissions(invoke: Invoke): void {
    this.checkPermissions(invoke);
  }

  private async checkPermissions(invoke: Invoke): Promise<void> {
    try {
      const isEnabled = await notificationManager.isNotificationEnabled();
      const state = isEnabled ? 'granted' : 'denied';
      invoke.resolve(JSON.stringify({ permissionState: state }));
    } catch (e) {
      const err = e as BusinessError;
      console.error('[NotificationPlugin] checkPermissions failed: ' + err.message);
      invoke.resolve(JSON.stringify({ permissionState: 'denied' }));
    }
  }

  // ─── createChannel ───

  private handleCreateChannel(invoke: Invoke): void {
    this.createChannel(invoke);
  }

  private async createChannel(invoke: Invoke): Promise<void> {
    try {
      const args: ChannelPayload = JSON.parse(invoke.parseArgs()) as ChannelPayload;
      const slotType = importanceToSlotType(args.importance);

      // If overwriting existing channel with different SlotType, decrement old refCount
      const oldConfig = this.channelMap.get(args.id);
      if (oldConfig != null && oldConfig.slotType !== slotType) {
        const oldCount = this.slotTypeRefCount.get(oldConfig.slotType) ?? 1;
        if (oldCount <= 1) {
          this.slotTypeRefCount.delete(oldConfig.slotType);
        } else {
          this.slotTypeRefCount.set(oldConfig.slotType, oldCount - 1);
        }
      }

      // Track SlotType usage — warn if shared with another channel
      const existingCount = this.slotTypeRefCount.get(slotType) ?? 0;
      if (existingCount > 0) {
        console.warn('[NotificationPlugin] SlotType ' + slotType + ' already used by ' + existingCount + ' channel(s). ' +
          'OHOS only has 4 SlotType values; channels sharing the same SlotType will overwrite each other\'s system config.');
      }
      this.slotTypeRefCount.set(slotType, existingCount + 1);

      // Create the slot in the system (uses default config)
      await notificationManager.addSlot(slotType);

      // Store full config in local mapping table
      const config: ChannelConfig = {
        id: args.id,
        name: args.name,
        description: args.description,
        importance: args.importance,
        sound: args.sound,
        lights: args.lights,
        lightColor: args.lightColor,
        vibration: args.vibration,
        visibility: args.visibility,
        slotType: slotType
      };
      this.channelMap.set(args.id, config);

      invoke.resolve('');
    } catch (e) {
      const err = e as BusinessError;
      console.error('[NotificationPlugin] createChannel failed: ' + err.message);
      invoke.reject('Create channel failed: ' + err.message);
    }
  }

  // ─── deleteChannel ───

  private handleDeleteChannel(invoke: Invoke): void {
    this.deleteChannel(invoke);
  }

  private async deleteChannel(invoke: Invoke): Promise<void> {
    try {
      const args: DeleteChannelArgs = JSON.parse(invoke.parseArgs()) as DeleteChannelArgs;

      const config = this.channelMap.get(args.id);
      if (config != null) {
        // Only remove system slot when no other channels share this SlotType
        const count = this.slotTypeRefCount.get(config.slotType) ?? 1;
        if (count <= 1) {
          await notificationManager.removeSlot(config.slotType);
          this.slotTypeRefCount.delete(config.slotType);
        } else {
          this.slotTypeRefCount.set(config.slotType, count - 1);
          console.warn('[NotificationPlugin] SlotType ' + config.slotType + ' still used by ' + (count - 1) + ' other channel(s), keeping system slot.');
        }
        this.channelMap.delete(args.id);
      }
      // Idempotent: no error if channel doesn't exist

      invoke.resolve('');
    } catch (e) {
      const err = e as BusinessError;
      console.error('[NotificationPlugin] deleteChannel failed: ' + err.message);
      invoke.reject('Delete channel failed: ' + err.message);
    }
  }

  // ─── listChannels ───

  private handleListChannels(invoke: Invoke): void {
    this.listChannels(invoke);
  }

  private async listChannels(invoke: Invoke): Promise<void> {
    try {
      const systemSlots = await notificationManager.getSlots();
      const channels: ChannelPayload[] = [];

      // Collect local configs into an array for iteration (avoid Map.forEach type issues in ArkTS)
      const localConfigs: ChannelConfig[] = [];
      const configKeys: string[] = [];
      for (const key of this.channelMap.keys()) {
        configKeys.push(key);
      }
      for (let i = 0; i < configKeys.length; i++) {
        const val = this.channelMap.get(configKeys[i]);
        if (val != null) {
          localConfigs.push(val);
        }
      }

      // Merge system slots with local mapping
      for (const slot of systemSlots) {
        // Find local config by matching slotType
        let matchedConfig: ChannelConfig | undefined = undefined;
        for (let j = 0; j < localConfigs.length; j++) {
          if (localConfigs[j].slotType === slot.notificationType) {
            matchedConfig = localConfigs[j];
            break;
          }
        }

        const ch: ChannelPayload = {
          id: matchedConfig != null ? matchedConfig.id : ('slot_' + slot.notificationType),
          name: matchedConfig != null ? matchedConfig.name : ('Channel ' + slot.notificationType),
        };
        // Always set importance to avoid undefined fields in JSON (serde deserialization requires it)
        ch.importance = matchedConfig != null && matchedConfig.importance != null ? matchedConfig.importance : 3; // Default importance
        if (matchedConfig != null) {
          ch.description = matchedConfig.description;
          ch.sound = matchedConfig.sound;
          ch.lights = matchedConfig.lights;
          ch.lightColor = matchedConfig.lightColor;
          ch.vibration = matchedConfig.vibration;
          ch.visibility = matchedConfig.visibility;
        }
        channels.push(ch);
      }

      // Include local-only channels that don't have a system slot yet
      for (let k = 0; k < localConfigs.length; k++) {
        const config = localConfigs[k];
        let found = false;
        for (let m = 0; m < channels.length; m++) {
          if (channels[m].id === config.id) {
            found = true;
            break;
          }
        }
        if (!found) {
          const localCh: ChannelPayload = {
            id: config.id,
            name: config.name,
          };
          localCh.importance = config.importance != null ? config.importance : 3;
          localCh.description = config.description;
          localCh.sound = config.sound;
          localCh.lights = config.lights;
          localCh.lightColor = config.lightColor;
          localCh.vibration = config.vibration;
          localCh.visibility = config.visibility;
          channels.push(localCh);
        }
      }

      invoke.resolve(JSON.stringify(channels));
    } catch (e) {
      const err = e as BusinessError;
      console.error('[NotificationPlugin] listChannels failed: ' + err.message);
      invoke.reject('List channels failed: ' + err.message);
    }
  }
}

export default NotificationPlugin;
