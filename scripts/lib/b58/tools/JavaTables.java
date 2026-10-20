// Prints Java 17's character tables for embedding in Grasm packages.
// `digit16`: Character.digit(c, 16) over the BMP as (lo, hi, value at lo),
// each range's values rising by one per character.
public class JavaTables {
    public static void main(String[] args) {
        StringBuilder b = new StringBuilder();
        int line = 0;
        int c = 0;
        java.util.List<Integer> out = new java.util.ArrayList<>();
        while (c <= 0xFFFF) {
            int d = Character.digit((char) c, 16);
            if (d < 0) { c++; continue; }
            int lo = c;
            while (c + 1 <= 0xFFFF && Character.digit((char) (c + 1), 16) == Character.digit((char) c, 16) + 1) c++;
            out.add(lo); out.add(c); out.add(d);
            c++;
        }
        for (int i = 0; i < out.size(); i++) {
            String s = Integer.toString(out.get(i));
            if (i > 0) {
                b.append(",");
                line++;
                if (line + s.length() > 120) { b.append("\n        "); line = 0; } else { b.append(" "); line++; }
            }
            b.append(s);
            line += s.length();
        }
        System.out.println("        " + b);
    }
}
