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

use super::{
    best_mosaic,
    ImageOffset,
    MosaicImageDims,
    scale_height_dimension,
    scale_width_dimension,
    Size,
    SPACING_SIZE,
};

pub(super) fn best_2_mosaic(first: Size, second: Size) -> MosaicImageDims<2> {
    let top_bottom = top_bottom_2_mosaic(first, second);
    let left_right = left_right_2_mosaic(first, second);
    return best_mosaic(&[&top_bottom, &left_right]);
}

pub fn left_right_2_mosaic(first: Size, second: Size) -> MosaicImageDims<2> {
    MosaicImageDims {
        images: [
            ImageOffset {
                offset: Size {
                    width: 0,
                    height: 0,
                },
                dimensions: first,
                original_dimensions: first,
            },
            ImageOffset {
                offset: Size {
                    width: first.width + SPACING_SIZE,
                    height: 0,
                },
                dimensions: scale_height_dimension(second, first.height),
                original_dimensions: second,
            },
        ]
    }
}

pub fn top_bottom_2_mosaic(first: Size, second: Size) -> MosaicImageDims<2> {
    MosaicImageDims {
        images: [
            ImageOffset {
                offset: Size {
                    width: 0,
                    height: 0,
                },
                dimensions: first,
                original_dimensions: first,
            },
            ImageOffset {
                offset: Size {
                    width: 0,
                    height: first.height + SPACING_SIZE,
                },
                dimensions: scale_width_dimension(second, first.width),
                original_dimensions: second,
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use crate::mosaic::testutils::mosaic;
    use crate::mosaic::testutils::{
        BLUE,
        create_with_colour,
        has_black_horizontal_line,
        has_black_vertical_line,
        is_colour_in_range,
        RED,
    };

    #[test]
    fn mosaic_2_left_right() {
        let left = create_with_colour(100, 400, RED);
        let right = create_with_colour(200, 400, BLUE);

        let result = mosaic(vec![left, right]);
        assert!(is_colour_in_range(0, 0, 100, 400, &result, RED));
        assert!(is_colour_in_range(120, 0, 300, 400, &result, BLUE));
        assert!(has_black_vertical_line(105, &result));
    }

    #[test]
    fn mosaic_2_top_bottom() {
        let top = create_with_colour(400, 200, RED);
        let bottom = create_with_colour(400, 100, BLUE);

        let result = mosaic(vec![top, bottom]);
        assert!(is_colour_in_range(0, 0, 400, 200, &result, RED));
        assert!(is_colour_in_range(0, 220, 400, 300, &result, BLUE));
        assert!(has_black_horizontal_line(205, &result));
    }
}