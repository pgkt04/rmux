/*
 * Dump the pinned tmux options_table[] and options_other_names[] in the
 * canonical text form compared by rmux-server options::table tests.
 * Build: cc -DHAVE_CLOCK_GETTIME -DHAVE_EVENT2_EVENT_H -DHAVE_SYS_QUEUE_H \
 *   -DHAVE_SYS_TREE_H -DHAVE_BITSTRING_H -DHAVE_U_INT -DHAVE_U_CHAR \
 *   -DHAVE_STRLCPY -DHAVE_STRLCAT -DHAVE_STRNLEN -DHAVE_STRNDUP \
 *   -DHAVE_SETPROCTITLE -D_FORTIFY_SOURCE=0 -DTMUX_MOUSE=1 \
 *   -I<pinned source> -I/opt/homebrew/opt/libevent/include \
 *   scripts/options-table-dump.c <pinned source>/options-table.c
 */
#include <stdio.h>
#include "tmux.h"

static void
put_str(const char *key, const char *s)
{
	printf("%s=", key);
	if (s == NULL) {
		printf("<NULL>\n");
		return;
	}
	for (; *s != '\0'; s++) {
		unsigned char c = (unsigned char)*s;
		if (c == '\\')
			printf("\\\\");
		else if (c < 0x20 || c >= 0x7f)
			printf("\\x%02x", c);
		else
			putchar(c);
	}
	putchar('\n');
}

static void
put_list(const char *key, const char **list)
{
	const char	**cp;
	char		  name[64];

	if (list == NULL) {
		printf("%s=<NULL>\n", key);
		return;
	}
	for (cp = list; *cp != NULL; cp++) {
		snprintf(name, sizeof name, "%s[%u]", key, (unsigned)(cp - list));
		put_str(name, *cp);
	}
	printf("%s.len=%u\n", key, (unsigned)(cp - list));
}

int
main(void)
{
	const struct options_table_entry	*oe;
	const struct options_name_map		*map;

	for (map = options_other_names; map->from != NULL; map++)
		printf("alias %s -> %s\n", map->from, map->to);
	for (oe = options_table; oe->name != NULL; oe++) {
		put_str("name", oe->name);
		printf("type=%d\n", (int)oe->type);
		printf("scope=%d\n", oe->scope);
		printf("flags=%d\n", oe->flags);
		printf("minimum=%u\n", oe->minimum);
		printf("maximum=%u\n", oe->maximum);
		put_list("choices", oe->choices);
		put_str("default_str", oe->default_str);
		printf("default_num=%lld\n", oe->default_num);
		put_list("default_arr", oe->default_arr);
		put_str("separator", oe->separator);
		put_str("pattern", oe->pattern);
		put_str("text", oe->text);
		put_str("unit", oe->unit);
		putchar('\n');
	}
	return (0);
}
