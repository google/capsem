/*
 * This file is part of Xpra.
 * Copyright (C) 2021 Tijs van der Zwaan <tijzwa@vpo.nl>
 * Copyright (c) 2021 Antoine Martin <antoine@xpra.org>
 * Licensed under MPL 2.0, see:
 * http://www.mozilla.org/MPL/2.0/
 *
 */

class XpraImageDecoder {
  async convertToBitmap(packet) {
var width = packet[4];
var height = packet[5];
var coding = packet[6];
    if (coding.startsWith("rgb")) {
var data = decode_rgb(packet);
var bitmap = await createImageBitmap(new ImageData(new Uint8ClampedArray(data.buffer), width, height), 0, 0, width, height);
      packet[6] = `bitmap:${coding}`;
      packet[7] = bitmap;
    } else {
var paint_coding = coding.split("/")[0]; //ie: "png/P" -> "png"
var options = packet[10];
var bitmap_options = {
        premultiplyAlpha: "none",
      };
      if ("scaled_size" in options) {
        bitmap_options.resizeWidth = width;
        bitmap_options.resizeHeight = height;
        bitmap_options.resizeQuality = options["scaling-quality"] || "medium";
      }

var blob = new Blob([packet[7].buffer], {
        type: `image/${paint_coding}`,
      });
var bitmap = await createImageBitmap(blob, bitmap_options);
      packet[6] = `bitmap:${coding}`;
      packet[7] = bitmap;
    }
    return packet;
  }
}