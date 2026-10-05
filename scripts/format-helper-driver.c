/*
 * Differential driver for G10 helper modules, linked against the pinned
 * tmux objects. Reads one case per line from stdin; byte arguments are hex.
 */
#include <sys/types.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <regex.h>
#include <locale.h>

#include "tmux.h"

static u_char *
unhex(const char *s, size_t *len)
{
	size_t	 n = strlen(s) / 2, i;
	u_char	*out = xmalloc(n + 1);
	unsigned u;

	for (i = 0; i < n; i++) {
		sscanf(s + 2 * i, "%2x", &u);
		out[i] = u;
	}
	out[n] = '\0';
	if (len != NULL)
		*len = n;
	return (out);
}

static void
hex(const u_char *s, size_t len)
{
	size_t	i;

	for (i = 0; i < len; i++)
		printf("%02x", s[i]);
}

static void
dump_cell(const struct grid_cell *gc)
{
	hex(gc->data.data, gc->data.size);
	printf(":%d:%d:%d:%x:%x:%u:%u", gc->fg, gc->bg, gc->us, gc->attr,
	    gc->flags, gc->data.width, gc->link);
}

static void
do_draw(char **argv)
{
	size_t			 len;
	u_char			*expanded = unhex(argv[0], &len);
	u_int			 available = atoi(argv[1]);
	u_int			 sx = atoi(argv[2]);
	u_int			 ocx = atoi(argv[3]);
	int			 default_colours = atoi(argv[4]);
	int			 want_ranges = atoi(argv[5]);
	struct grid_cell	 base;
	struct screen		 s;
	struct screen_write_ctx	 ctx;
	struct style_ranges	 srs;
	struct style_range	*sr;
	struct grid_cell	 gc;
	u_int			 x;
	const char		*uri, *id;

	memcpy(&base, &grid_default_cell, sizeof base);
	base.fg = atoi(argv[6]);
	base.bg = atoi(argv[7]);
	TAILQ_INIT(&srs);

	screen_init(&s, sx, 1, 0);
	screen_write_start(&ctx, &s);
	screen_write_cursormove(&ctx, ocx, 0, 0);
	format_draw(&ctx, &base, available, expanded,
	    want_ranges ? &srs : NULL, default_colours);
	screen_write_stop(&ctx);

	for (x = 0; x < sx; x++) {
		grid_view_get_cell(s.grid, x, 0, &gc);
		if (x != 0)
			printf(" ");
		dump_cell(&gc);
		if (gc.link != 0 &&
		    hyperlinks_get(s.hyperlinks, gc.link, &uri, &id, NULL)) {
			printf("=");
			hex(uri, strlen(uri));
			printf("=");
			hex(id, strlen(id));
		}
	}
	printf("|%u,%u|", s.cx, s.cy);
	TAILQ_FOREACH(sr, &srs, entry) {
		printf("%d:%u:", sr->type, sr->argument);
		hex(sr->string, strlen(sr->string));
		printf(":%u-%u;", sr->start, sr->end);
	}
	printf("\n");
	screen_free(&s);
	free(expanded);
}

int
main(int argc, char **argv)
{
	char	 line[65536], *argvv[16], *cp, *tok;
	int	 n;

	(void)argc;
	(void)argv;
	setlocale(LC_CTYPE, "en_US.UTF-8");
	global_options = options_create(NULL);
	global_s_options = options_create(NULL);
	global_w_options = options_create(NULL);
	for (const struct options_table_entry *oe = options_table;
	    oe->name != NULL; oe++) {
		if (oe->scope & OPTIONS_TABLE_SERVER)
			options_default(global_options, oe);
		if (oe->scope & OPTIONS_TABLE_SESSION)
			options_default(global_s_options, oe);
		if (oe->scope & OPTIONS_TABLE_WINDOW)
			options_default(global_w_options, oe);
	}
	while (fgets(line, sizeof line, stdin) != NULL) {
		line[strcspn(line, "\n")] = '\0';
		n = 0;
		cp = line;
		while ((tok = strsep(&cp, " ")) != NULL && n < 16)
			argvv[n++] = tok;
		if (n == 0)
			continue;
		if (strcmp(argvv[0], "regsub") == 0 && n == 5) {
			u_char	*pattern = unhex(argvv[1], NULL);
			u_char	*with = unhex(argvv[2], NULL);
			u_char	*text = unhex(argvv[3], NULL);
			int	 flags = atoi(argvv[4]);
			char	*out = regsub(pattern, with, text, flags);

			if (out == NULL)
				printf("ERR\n");
			else {
				printf("OK ");
				hex(out, strlen(out));
				printf("\n");
				free(out);
			}
			free(pattern); free(with); free(text);
		} else if (strcmp(argvv[0], "fuzzy") == 0 && n == 4) {
			u_char		*pattern = unhex(argvv[1], NULL);
			u_char		*text = unhex(argvv[2], NULL);
			u_int		 width = atoi(argvv[3]), score = 0, i;
			bitstr_t	*bs = fuzzy_match(pattern, text, width,
					     &score);
			int		 first = 1;

			if (bs == NULL)
				printf("NONE\n");
			else {
				printf("%u", score);
				for (i = 0; i < width; i++) {
					if (!bit_test(bs, i))
						continue;
					printf("%c%u", first ? ' ' : ',', i);
					first = 0;
				}
				printf("\n");
				free(bs);
			}
			free(pattern); free(text);
		} else if (strcmp(argvv[0], "json") == 0 && n == 2) {
			u_char			*input = unhex(argvv[1], NULL);
			char			*cause = NULL, *out;
			struct json_node	*jn = json_parse(input, &cause);

			if (jn == NULL) {
				printf("ERR ");
				hex(cause, strlen(cause));
				printf("\n");
				free(cause);
			} else {
				out = json_to_string(jn);
				printf("OK ");
				hex(out, strlen(out));
				printf("\n");
				free(out);
				json_destroy_node(jn);
			}
			free(input);
		} else if (strcmp(argvv[0], "width") == 0 && n == 2) {
			u_char	*s = unhex(argvv[1], NULL);

			printf("%u\n", format_width(s));
			free(s);
		} else if (strcmp(argvv[0], "trimleft") == 0 && n == 3) {
			u_char	*s = unhex(argvv[1], NULL);
			char	*out = format_trim_left(s, atoi(argvv[2]));

			hex(out, strlen(out));
			printf("\n");
			free(out); free(s);
		} else if (strcmp(argvv[0], "trimright") == 0 && n == 3) {
			u_char	*s = unhex(argvv[1], NULL);
			char	*out = format_trim_right(s, atoi(argvv[2]));

			hex(out, strlen(out));
			printf("\n");
			free(out); free(s);
		} else if (strcmp(argvv[0], "draw") == 0 && n == 9) {
			do_draw(argvv + 1);
		} else
			printf("BAD\n");
		fflush(stdout);
	}
	return (0);
}
