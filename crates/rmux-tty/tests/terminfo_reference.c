// Ported from tmux tty-term.c @ 8f25579c
/* $OpenBSD: tty-term.c,v 1.111 2026/09/22 14:10:26 nicm Exp $ */

/*
 * Copyright (c) 2008 Nicholas Marriott <nicholas.marriott@gmail.com>
 *
 * Permission to use, copy, modify, and distribute this software for any
 * purpose with or without fee is hereby granted, provided that the above
 * copyright notice and this permission notice appear in all copies.
 *
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
 * ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF MIND, USE, DATA OR PROFITS, WHETHER
 * IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING
 * OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <curses.h>
#include <term.h>

static const struct { const char *name; int kind; } codes[] = {
    {"acsc", 1},
    {"am", 3},
    {"AX", 3},
    {"bce", 3},
    {"bel", 1},
    {"Bidi", 1},
    {"blink", 1},
    {"bold", 1},
    {"civis", 1},
    {"clear", 1},
    {"Clmg", 1},
    {"Cmg", 1},
    {"cnorm", 1},
    {"colors", 2},
    {"Cr", 1},
    {"Cs", 1},
    {"csr", 1},
    {"cub", 1},
    {"cub1", 1},
    {"cud", 1},
    {"cud1", 1},
    {"cuf", 1},
    {"cuf1", 1},
    {"cup", 1},
    {"cuu", 1},
    {"cuu1", 1},
    {"cvvis", 1},
    {"dch", 1},
    {"dch1", 1},
    {"dim", 1},
    {"dl", 1},
    {"dl1", 1},
    {"Dsbp", 1},
    {"Dseks", 1},
    {"Dsesc", 1},
    {"Dsfcs", 1},
    {"Dsmg", 1},
    {"E3", 1},
    {"ech", 1},
    {"ed", 1},
    {"el", 1},
    {"el1", 1},
    {"enacs", 1},
    {"Enbp", 1},
    {"Eneks", 1},
    {"Enesc", 1},
    {"Enfcs", 1},
    {"Enmg", 1},
    {"fsl", 1},
    {"Hls", 1},
    {"home", 1},
    {"hpa", 1},
    {"ich", 1},
    {"ich1", 1},
    {"il", 1},
    {"il1", 1},
    {"ind", 1},
    {"indn", 1},
    {"invis", 1},
    {"kcbt", 1},
    {"kcub1", 1},
    {"kcud1", 1},
    {"kcuf1", 1},
    {"kcuu1", 1},
    {"kDC", 1},
    {"kDC3", 1},
    {"kDC4", 1},
    {"kDC5", 1},
    {"kDC6", 1},
    {"kDC7", 1},
    {"kdch1", 1},
    {"kDN", 1},
    {"kDN3", 1},
    {"kDN4", 1},
    {"kDN5", 1},
    {"kDN6", 1},
    {"kDN7", 1},
    {"kend", 1},
    {"kEND", 1},
    {"kEND3", 1},
    {"kEND4", 1},
    {"kEND5", 1},
    {"kEND6", 1},
    {"kEND7", 1},
    {"kf1", 1},
    {"kf10", 1},
    {"kf11", 1},
    {"kf12", 1},
    {"kf13", 1},
    {"kf14", 1},
    {"kf15", 1},
    {"kf16", 1},
    {"kf17", 1},
    {"kf18", 1},
    {"kf19", 1},
    {"kf2", 1},
    {"kf20", 1},
    {"kf21", 1},
    {"kf22", 1},
    {"kf23", 1},
    {"kf24", 1},
    {"kf25", 1},
    {"kf26", 1},
    {"kf27", 1},
    {"kf28", 1},
    {"kf29", 1},
    {"kf3", 1},
    {"kf30", 1},
    {"kf31", 1},
    {"kf32", 1},
    {"kf33", 1},
    {"kf34", 1},
    {"kf35", 1},
    {"kf36", 1},
    {"kf37", 1},
    {"kf38", 1},
    {"kf39", 1},
    {"kf4", 1},
    {"kf40", 1},
    {"kf41", 1},
    {"kf42", 1},
    {"kf43", 1},
    {"kf44", 1},
    {"kf45", 1},
    {"kf46", 1},
    {"kf47", 1},
    {"kf48", 1},
    {"kf49", 1},
    {"kf5", 1},
    {"kf50", 1},
    {"kf51", 1},
    {"kf52", 1},
    {"kf53", 1},
    {"kf54", 1},
    {"kf55", 1},
    {"kf56", 1},
    {"kf57", 1},
    {"kf58", 1},
    {"kf59", 1},
    {"kf6", 1},
    {"kf60", 1},
    {"kf61", 1},
    {"kf62", 1},
    {"kf63", 1},
    {"kf7", 1},
    {"kf8", 1},
    {"kf9", 1},
    {"kHOM", 1},
    {"kHOM3", 1},
    {"kHOM4", 1},
    {"kHOM5", 1},
    {"kHOM6", 1},
    {"kHOM7", 1},
    {"khome", 1},
    {"kIC", 1},
    {"kIC3", 1},
    {"kIC4", 1},
    {"kIC5", 1},
    {"kIC6", 1},
    {"kIC7", 1},
    {"kich1", 1},
    {"kind", 1},
    {"kLFT", 1},
    {"kLFT3", 1},
    {"kLFT4", 1},
    {"kLFT5", 1},
    {"kLFT6", 1},
    {"kLFT7", 1},
    {"kmous", 1},
    {"knp", 1},
    {"kNXT", 1},
    {"kNXT3", 1},
    {"kNXT4", 1},
    {"kNXT5", 1},
    {"kNXT6", 1},
    {"kNXT7", 1},
    {"kpp", 1},
    {"kPRV", 1},
    {"kPRV3", 1},
    {"kPRV4", 1},
    {"kPRV5", 1},
    {"kPRV6", 1},
    {"kPRV7", 1},
    {"kri", 1},
    {"kRIT", 1},
    {"kRIT3", 1},
    {"kRIT4", 1},
    {"kRIT5", 1},
    {"kRIT6", 1},
    {"kRIT7", 1},
    {"kUP", 1},
    {"kUP3", 1},
    {"kUP4", 1},
    {"kUP5", 1},
    {"kUP6", 1},
    {"kUP7", 1},
    {"Ms", 1},
    {"Nobr", 1},
    {"ol", 1},
    {"op", 1},
    {"Rect", 1},
    {"rev", 1},
    {"RGB", 3},
    {"ri", 1},
    {"rin", 1},
    {"rmacs", 1},
    {"rmcup", 1},
    {"rmkx", 1},
    {"Se", 1},
    {"setab", 1},
    {"setaf", 1},
    {"setal", 1},
    {"setrgbb", 1},
    {"setrgbf", 1},
    {"Setulc", 1},
    {"Setulc1", 1},
    {"sgr0", 1},
    {"sitm", 1},
    {"smacs", 1},
    {"smcup", 1},
    {"smkx", 1},
    {"Smol", 1},
    {"smso", 1},
    {"smul", 1},
    {"Smulx", 1},
    {"smxx", 1},
    {"Spb", 1},
    {"Sxl", 3},
    {"Ss", 1},
    {"Swd", 1},
    {"Sync", 1},
    {"Tc", 3},
    {"tsl", 1},
    {"U8", 2},
    {"vpa", 1},
    {"XT", 3},
};

static void bytes(const char *value) {
    if (value == NULL) { puts("NULL"); return; }
    for (const unsigned char *p = (const unsigned char *)value; *p; ++p)
        printf("%02x", *p);
    putchar('\n');
}

int main(int argc, char **argv) {
    if (argc < 3) return 2;
    if (!strcmp(argv[1], "caps")) {
        int error = 99;
        if (setupterm(argv[2], -1, &error) != OK) {
            printf("ERR %d\n", error);
            return 0;
        }
        puts("OK");
        for (unsigned i = 0; i < sizeof(codes) / sizeof(codes[0]); ++i) {
            char *value;
            int number;
            if (codes[i].kind == 1) {
                value = tigetstr(codes[i].name);
                if (value == NULL || value == (char *)-1) continue;
                printf("%s=", codes[i].name);
                bytes(value);
            } else {
                number = codes[i].kind == 2 ? tigetnum(codes[i].name) : tigetflag(codes[i].name);
                if (number < 0) continue;
                printf("%s=%d\n", codes[i].name, number);
            }
        }
        return 0;
    }
    if (!strcmp(argv[1], "checks")) {
        bytes(tiparm_s(1, 0, "constant", 1));
        bytes(tiparm_s(1, 0, "%p2%d", 1));
        bytes(tiparm_s(1, 0, "%p1%s", 1));
        bytes(tiparm_s(1, 1, "%p1%d", "x"));
        return 0;
    }
    if (!strcmp(argv[1], "parm")) {
        int error, expected, type;
        setupterm("xterm-256color", -1, &error);
        if (tiscan_s(&expected, &type, argv[2]) != OK) return 3;
        if (argc >= 5 && !strcmp(argv[3], "ss"))
            bytes(tiparm_s(expected, type, argv[2], argv[4], argc > 5 ? argv[5] : ""));
        else {
            int p[9] = {0};
            for (int i = 3; i < argc && i < 12; ++i) p[i-3] = (int)strtol(argv[i], NULL, 10);
            bytes(tiparm_s(expected, type, argv[2], p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7], p[8]));
        }
        return 0;
    }
    if (!strcmp(argv[1], "vars")) {
        int error;
        setupterm("xterm-256color", -1, &error);
        bytes(tiparm_s(0, 0, "%{42}%PA%{9}%Pa%gA%d:%ga%d"));
        bytes(tiparm_s(0, 0, "%gA%d:%ga%d"));
        return 0;
    }
    return 2;
}
