-- PH-203: server-side search normalization for the narrative review queue.
--
-- The queue searches linked person names, section titles, and passage bodies.
-- Search must ignore case and Vietnamese diacritics, including đ -> d. The
-- database does not install the unaccent extension, so this function uses
-- Unicode NFD decomposition and removes the combining-mark block explicitly.
-- It is read-only and never changes stored text.

BEGIN;

CREATE OR REPLACE FUNCTION narrative_search_normalize(input text)
RETURNS text
LANGUAGE sql
IMMUTABLE
PARALLEL SAFE
AS $$
    SELECT replace(
        translate(
            normalize(lower(coalesce(input, '')), NFD),
            U&'\0300\0301\0302\0303\0304\0305\0306\0307\0308\0309\030a\030b\030c\030d\030e\030f\0310\0311\0312\0313\0314\0315\0316\0317\0318\0319\031a\031b\031c\031d\031e\031f\0320\0321\0322\0323\0324\0325\0326\0327\0328\0329\032a\032b\032c\032d\032e\032f\0330\0331\0332\0333\0334\0335\0336\0337\0338\0339\033a\033b\033c\033d\033e\033f\0340\0341\0342\0343\0344\0345\0346\0347\0348\0349\034a\034b\034c\034d\034e\034f\0350\0351\0352\0353\0354\0355\0356\0357\0358\0359\035a\035b\035c\035d\035e\035f\0360\0361\0362\0363\0364\0365\0366\0367\0368\0369\036a\036b\036c\036d\036e\036f',
            ''
        ),
        'đ',
        'd'
    )
$$;

COMMIT;
