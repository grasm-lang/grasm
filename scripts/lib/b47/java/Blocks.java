// Dumps java.lang.Character.UnicodeBlock as ranges: `start end NAME`, one a
// line, for every block a code point belongs to.
public class Blocks {
    public static void main(String[] a) {
        Character.UnicodeBlock cur = null;
        int start = 0;
        for (int cp = 0; cp <= 0x110000; cp++) {
            Character.UnicodeBlock b = cp <= 0x10FFFF ? Character.UnicodeBlock.of(cp) : null;
            if (b != cur) {
                if (cur != null) System.out.println(Integer.toHexString(start) + " " + Integer.toHexString(cp - 1) + " " + cur);
                cur = b;
                start = cp;
            }
        }
    }
}
