# Kaku

[![Crates.io Version](https://img.shields.io/crates/v/kaku)](https://crates.io/crates/kaku)
[![docs.rs](https://img.shields.io/docsrs/kaku)](https://docs.rs/kaku/latest/kaku/)

A text rendering crate for rust+wgpu, with the ability to use signed distance fields.

## Features

- Rendering of OpenType fonts (loaded with [ab_glyph](https://github.com/alexheretic/ab-glyph)).
- Simple, non-SDF text rendering for performance (see "Performance").
- SDF-based text rendering for high quality upscaling and fast outlining.

## What do signed distance fields do?

Without going into details, signed distance fields are a way of representing a shape (such as a character in a font) in a way that allows for high quality upscaling, reducing memory usage. It also allows you to render certain effects such as outlines in a way that is very performant.

## Performance

When you create a Text object with kaku, it has to generate the signed distance field for every new glyph in the text. These fields are cached, so this only ever has to happen once per glyph per font. Calculating this texture takes a small but not-insignificant amount of time (on the order of 1ms in my testing, but this will depend on your computer and on the glyph), so if this is a problem, you can also render text without sdf.

You can also pre-compute the distance fields for characters you know that you will draw. For example, for an English-language video game, you could cache all alphanumeric characters in the startup loading screen.

Once a text object is created, rendering it to the screen is about as fast with sdf as it is without.

## Example

Here is a screenshot of the demo example, showing some of the things kaku can do:

![Example of kaku rendering text](https://github.com/villuna/kaku/blob/main/images/demo.png)

This example shows the same text:

- Rendered with no SDF, using just the textures provided by ab_glyph.
- Rendered with SDF.
- Rendered with SDF, with a large outline.
- Rendered with SDF, upscaled 2x. Even though it's upscaled, it's still crisp and smooth!
