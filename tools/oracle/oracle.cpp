// Differential-test driver around the ORIGINAL C++ ukengine.
// Reads a script from stdin, prints what the engine outputs per token.
// Tokens per line:  CFG <im> <free> <modern> <spell> <restore>  |  RST | BS | CAPS0 | CAPS1 | <hex key>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include "keycons.h"
#include "vnconv.h"
#include "unikey.h"

static void emit()
{
    printf("%d %d ", UnikeyBackspaces, UnikeyOutput == UkKeyOutput ? 1 : 0);
    if (UnikeyOutput == UkKeyOutput) {
        for (int i = 0; i < UnikeyBufChars; i++) printf("%04x", UnikeyBuf[i]);
    } else {
        for (int i = 0; i + 1 < UnikeyBufChars; i += 2) printf("%04x", UnikeyBuf[i] | (UnikeyBuf[i + 1] << 8));
    }
    printf("\n");
}

int main()
{
    UnikeySetup();
    UnikeySetOutputCharset(CONV_CHARSET_UNICODE);
    static char line[1 << 20];
    while (fgets(line, sizeof line, stdin)) {
        char *tok = strtok(line, " \r\n");
        if (!tok) continue;
        if (!strcmp(tok, "CFG")) {
            int im = atoi(strtok(NULL, " \r\n"));
            UnikeyOptions opt;
            CreateDefaultUnikeyOptions(&opt);
            opt.freeMarking = atoi(strtok(NULL, " \r\n"));
            opt.modernStyle = atoi(strtok(NULL, " \r\n"));
            opt.spellCheckEnabled = atoi(strtok(NULL, " \r\n"));
            opt.autoNonVnRestore = atoi(strtok(NULL, " \r\n"));
            UnikeySetInputMethod((UkInputMethod)im);
            UnikeySetOptions(&opt);
            printf("CFG\n");
            continue;
        }
        for (; tok; tok = strtok(NULL, " \r\n")) {
            if (!strcmp(tok, "RST")) { UnikeyResetBuf(); printf("R\n"); }
            else if (!strcmp(tok, "CAPS0")) { UnikeySetCapsState(0, 0); printf("C\n"); }
            else if (!strcmp(tok, "CAPS1")) { UnikeySetCapsState(0, 1); printf("C\n"); }
            else if (!strcmp(tok, "BS")) { UnikeyBackspacePress(); emit(); }
            else { UnikeyFilter((unsigned int)strtoul(tok, NULL, 16)); emit(); }
        }
    }
    return 0;
}
