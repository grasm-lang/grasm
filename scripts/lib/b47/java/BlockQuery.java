// Answers `of <hex code point>` and `name <hex utf-8>` lines the way
// java.lang.Character.UnicodeBlock does: the block's constant name, or
// `null` (no block) / `error` (forName refused it). Each case file named on
// stdin gives one line of answers joined by `;`.
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;

public class BlockQuery {
    static String unhex(String h) {
        if (h.equals("-")) return "";
        byte[] b = new byte[h.length() / 2];
        for (int i = 0; i < b.length; i++) b[i] = (byte) Integer.parseInt(h.substring(2 * i, 2 * i + 2), 16);
        return new String(b, StandardCharsets.UTF_8);
    }
    public static void main(String[] a) throws Exception {
        BufferedReader in = new BufferedReader(new InputStreamReader(System.in));
        String path;
        while ((path = in.readLine()) != null) {
            if (path.isEmpty()) continue;
            List<String> out = new ArrayList<>();
            for (String line : Files.readAllLines(Paths.get(path))) {
                if (line.isEmpty()) continue;
                String[] f = line.split(" ");
                String ans;
                try {
                    if (f[0].equals("of")) {
                        int cp = (int) Long.parseLong(f[1], 16);
                        Character.UnicodeBlock b = Character.UnicodeBlock.of(cp);
                        ans = b == null ? "null" : b.toString();
                    } else {
                        ans = Character.UnicodeBlock.forName(unhex(f[1])).toString();
                    }
                } catch (IllegalArgumentException e) {
                    ans = "error";
                }
                out.add(ans);
            }
            System.out.println(String.join(";", out));
        }
    }
}
