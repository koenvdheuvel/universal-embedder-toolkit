/*
 * MIT License
 *
 * Copyright (c) 2022 Antonio32A (antonio32a.com) <~@antonio32a.com>
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in all
 * copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 * AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 * SOFTWARE.
 *
 * Adapted from https://github.com/FxEmbed/mosaic
 */

use image::{Rgb, RgbImage};

use super::render;

pub const BLACK: Rgb<u8> = Rgb([0, 0, 0]);
pub const RED: Rgb<u8> = Rgb([255, 0, 0]);
pub const BLUE: Rgb<u8> = Rgb([0, 0, 255]);
pub const GREEN: Rgb<u8> = Rgb([0, 255, 0]);
pub const PURPLE: Rgb<u8> = Rgb([255, 64, 255]);
pub fn create_with_colour(width: u32, height: u32, colour: Rgb<u8>) -> RgbImage {
    let mut img = RgbImage::new(width, height);

    for x in 0..width {
        for y in 0..height {
            img.put_pixel(x, y, colour);
            img.put_pixel(x, y, colour);
        }
    }

    img
}

pub fn is_colour_at_pixel(x: u32, y: u32, image: &RgbImage, colour: Rgb<u8>) -> bool {
    image.get_pixel(x, y).eq(&colour)
}

pub fn is_colour_in_range(start_x: u32, start_y: u32, end_x: u32, end_y: u32, image: &RgbImage, colour: Rgb<u8>) -> bool {
    for x in start_x..end_x {
        for y in start_y..end_y {
            if !is_colour_at_pixel(x, y, image, colour) {
                return false;
            }
        }
    }
    true
}

pub fn has_black_vertical_line(x: u32, image: &RgbImage) -> bool {
    is_colour_in_range(x, 0, x, image.height(), image, BLACK)
}

pub fn has_black_horizontal_line(y: u32, image: &RgbImage) -> bool {
    is_colour_in_range(0, y, image.width(), y, image, BLACK)
}

pub fn has_black_vertical_line_partial(x: u32, start_y: u32, end_y: u32, image: &RgbImage) -> bool {
    is_colour_in_range(x, start_y, x, end_y, image, BLACK)
}

pub fn has_black_horizontal_line_partial(y: u32, start_x: u32, end_x: u32, image: &RgbImage) -> bool {
    is_colour_in_range(start_x, y, end_x, y, image, BLACK)
}

pub fn mosaic(images: Vec<RgbImage>) -> RgbImage {
    render(images).expect("2-4 non-empty images")
}
