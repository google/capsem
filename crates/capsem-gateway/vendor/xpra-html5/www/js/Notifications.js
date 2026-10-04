/*
 * This file is part of Xpra.
 * Copyright (C) 2016 Antoine Martin <antoine@xpra.org>
 * Copyright (c) 2016 Spikes, Inc.
 * Licensed under MPL 2.0, see:
 * http://www.mozilla.org/MPL/2.0/
 *
 */

$(function() {
  window.notification_timers = {};

var NOTIFICATION_SELECTOR = ".notifications";

  window.doNotification = function(
    type,
    nid,
    title,
    message,
    timeout,
    icon,
    actions,
    hints,
    onAction,
    onClose
  ) {
    console.debug(
      "doNotification",
      type,
      nid,
      title,
      message,
      timeout,
      icon,
      actions,
      hints,
      onAction,
      onClose
    );
var nID = `notification${nid}`;
var a = $(
      `<div id="${nID}" class="alert ${type}">` +
      `<img class="notification_icon" id="notification_icon${nID}">` +
      `<span class="title">${title}</span>` +
      `<span class="message">${message}</span>` +
      `<div class="dismiss">&#215;</div>` +
      `</div>`
    );
var notifications_elements = $(NOTIFICATION_SELECTOR);
    notifications_elements.prepend(a);
    if (actions) {
var notification_buttons = $(
        '<div class="notification_buttons"></div>'
      );
      a.append(notification_buttons);
for(var index = 0; index < actions.length; index += 2) {
var action_id = actions[index];
var action_label = actions[index + 1];
var notification_button = window._notification_button(
          nid,
          action_id,
          action_label,
          onAction,
          onClose
        );
        notification_buttons.append(notification_button);
      }
    }
    notifications_elements.prepend(a);
    if (icon) {
var encoding = icon[0];
var img_data = icon[3];
      if (encoding == "png") {
var source = `data:image/${encoding};base64,${Utilities.ArrayBufferToBase64(
          img_data
        )}`;
        $(`#notification_icon${nID}`).attr("src", source);
      }
    }

    a.on("click", ".dismiss", function() {
      window.cancelNotificationTimer(nid);
      a.removeClass("visible").addClass("hidden");
      a.on(
        "transitionend webkitTransitionEnd",
        $.debounce(250, function() {
          a.trigger("dismissed");
          a.remove();
        })
      );
      if (onClose) {
        onClose(nid, 3, "user clicked dismiss");
      }
    });

    setTimeout(function() {
      a.addClass("visible");
    }, 1);

    if (timeout) {
      a.data("timeLeft", timeout);
      window.notification_timers[nid] = setInterval(function() {
var tleft = a.data("timeLeft") - 1;
        if (a.data("timeLeft") === 0) {
          a.find(".dismiss").trigger("click");
          if (onClose) {
            onClose(nid, 1, "timeout");
          }
        } else {
          a.find("sec").text(tleft);
          a.data("timeLeft", tleft);
        }
      }, 1000);
    }
    return a;
  };

  window._notification_button = function(
    nid,
    action_id,
    action_label,
    onAction,
    onClose
  ) {
var notification_button = $(
      `<div class="notification_button" id=notification"${action_id}">${action_label}</div>`
    );
    notification_button.on("click", function() {
      window.closeNotification(nid);
      if (onAction) {
        onAction(nid, action_id);
      }
      if (onClose) {
        onClose(nid, 3, "user clicked action");
      }
    });
    return notification_button;
  };

  window.cancelNotificationTimer = function(nid) {
var timer = window.notification_timers[nid];
    if (timer) {
      window.clearInterval(timer);
      delete window.notification_timers[nid];
    }
  };
  window.cancelNotificationTimers = function() {
for(var nid in window.notification_timers) {
      window.cancelNotificationTimer(nid);
    }
  };

  window.closeNotification = function(nid) {
    window.cancelNotificationTimer(nid);
var nID = `notification${nid}`;
    $(NOTIFICATION_SELECTOR).find(`#${nID}`).find(".dismiss").trigger("click");
  };

  window.clearNotifications = function() {
    window.cancelNotificationTimers();
    $(NOTIFICATION_SELECTOR).find(".dismiss").trigger("click");
  };

  window.removeNotifications = function() {
    window.cancelNotificationTimers();
    $(NOTIFICATION_SELECTOR).empty();
  };
});