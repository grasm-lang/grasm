// Dumps the names java.lang.Character.UnicodeBlock.forName accepts, as
// `KEY BLOCK` lines: the private map it looks names up in, upper-cased.
import java.lang.reflect.Field;
import java.util.Map;
import java.util.TreeMap;

public class BlockNames {
    public static void main(String[] a) throws Exception {
        Field f = Character.UnicodeBlock.class.getDeclaredField("map");
        f.setAccessible(true);
        @SuppressWarnings("unchecked")
        Map<String, Character.UnicodeBlock> m = (Map<String, Character.UnicodeBlock>) f.get(null);
        for (Map.Entry<String, Character.UnicodeBlock> e : new TreeMap<>(m).entrySet()) {
            System.out.println(e.getKey().replace(' ', '~') + " " + e.getValue());
        }
    }
}
