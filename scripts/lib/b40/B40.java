// Oracle for scripts/b40_diff.py: JAR manifests read and written by
// java.util.jar.Manifest, and object graphs written by ObjectOutputStream
// and dumped from what ObjectInputStream reads back.
import java.io.*;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.jar.*;

public class B40 {
    static String hex(byte[] b) {
        StringBuilder sb = new StringBuilder();
        for (byte x : b) sb.append(String.format("%02x", x & 255));
        return sb.toString();
    }
    static String hs(String s) { return hex(s.getBytes(StandardCharsets.UTF_8)); }

    // --- manifests ---------------------------------------------------------

    static String attrs(Attributes a) {
        List<String> out = new ArrayList<>();
        for (Map.Entry<Object, Object> e : a.entrySet()) out.add(hs(e.getKey().toString()) + "=" + hs((String) e.getValue()));
        return String.join(",", out);
    }

    static void manifests(String[] files) throws Exception {
        for (String f : files) {
            byte[] data = Files.readAllBytes(Paths.get(f));
            try {
                Manifest m = new Manifest(new ByteArrayInputStream(data));
                List<String> secs = new ArrayList<>();
                List<String> names = new ArrayList<>(m.getEntries().keySet());
                Collections.sort(names);
                for (String n : names) secs.add(hs(n) + ":" + attrs(m.getEntries().get(n)));
                String written = "-";
                if (m.getEntries().size() <= 1 && (m.getMainAttributes().getValue("Manifest-Version") != null
                        || m.getMainAttributes().getValue("Signature-Version") != null)) {
                    ByteArrayOutputStream o = new ByteArrayOutputStream();
                    m.write(o);
                    written = hex(o.toByteArray());
                }
                System.out.println(attrs(m.getMainAttributes()) + "|" + String.join(";", secs) + "|" + written);
            } catch (Exception e) {
                System.out.println("ERR");
            }
        }
    }

    // --- serialization -------------------------------------------------------

    enum Color { RED, GREEN, BLUE }

    static class P implements Serializable {
        private static final long serialVersionUID = 7L;
        int i; long j; short sh; byte b; char c; boolean z; float f; double d;
        String s; Object o; int[] arr; P next;
        transient int skipped = 5;
    }

    static class Q extends P {
        private static final long serialVersionUID = 8L;
        String extra; List<Object> list;
    }

    static class W implements Serializable {
        private static final long serialVersionUID = 9L;
        int x; String tag;
        private void writeObject(ObjectOutputStream out) throws IOException {
            out.defaultWriteObject();
            out.writeInt(x * 3);
            out.writeObject("note-" + x);
        }
        private void readObject(ObjectInputStream in) throws IOException, ClassNotFoundException {
            in.defaultReadObject();
            in.readInt();
            in.readObject();
        }
    }

    public static class E implements Externalizable {
        int v; String w;
        public E() {}
        public void writeExternal(ObjectOutput out) throws IOException { out.writeInt(v); out.writeUTF(w); out.writeObject(Color.GREEN); }
        public void readExternal(ObjectInput in) throws IOException, ClassNotFoundException { v = in.readInt(); w = in.readUTF(); in.readObject(); }
    }

    record R(int a, String b, double[] c) implements Serializable {}

    static Random rng;

    static String text() {
        int n = rng.nextInt(12);
        StringBuilder sb = new StringBuilder();
        for (int k = 0; k < n; k++) {
            int kind = rng.nextInt(6);
            if (kind == 0) sb.append('\0');
            else if (kind == 1) sb.appendCodePoint(0x1F600 + rng.nextInt(40));
            else if (kind == 2) sb.append((char) (0x400 + rng.nextInt(200)));
            else sb.append((char) ('a' + rng.nextInt(26)));
        }
        if (rng.nextInt(40) == 0) sb.append("x".repeat(70000));
        return sb.toString();
    }

    static Object leaf(int depth) {
        switch (rng.nextInt(depth > 3 ? 9 : 16)) {
            case 0: return null;
            case 1: return text();
            case 2: return rng.nextInt();
            case 3: return rng.nextLong();
            case 4: return rng.nextDouble() * 1e6;
            case 5: return rng.nextBoolean();
            case 6: return (char) ('A' + rng.nextInt(26));
            case 7: return Color.values()[rng.nextInt(3)];
            case 8: return new int[] { rng.nextInt(), rng.nextInt(100) };
            case 9: { P p = makeP(depth + 1); if (rng.nextInt(3) == 0) p.next = p; return p; }
            case 10: { ArrayList<Object> l = new ArrayList<>(); for (int k = rng.nextInt(4); k > 0; k--) l.add(leaf(depth + 1)); return l; }
            case 11: { HashMap<String, Object> m = new HashMap<>(); for (int k = rng.nextInt(3); k > 0; k--) m.put("k" + k, leaf(depth + 1)); return m; }
            case 12: { W w = new W(); w.x = rng.nextInt(1000); w.tag = text(); return w; }
            case 13: { E e = new E(); e.v = rng.nextInt(); e.w = "ext"; return e; }
            case 14: return new R(rng.nextInt(), text(), new double[] { rng.nextGaussian() });
            default: return new String[] { text(), null, "shared" };
        }
    }

    static P makeP(int depth) {
        P p = rng.nextBoolean() ? new P() : new Q();
        p.i = rng.nextInt(); p.j = rng.nextLong(); p.sh = (short) rng.nextInt(); p.b = (byte) rng.nextInt();
        p.c = (char) rng.nextInt(65536); p.z = rng.nextBoolean(); p.f = rng.nextFloat() - 0.5f; p.d = rng.nextGaussian();
        if (rng.nextInt(20) == 0) p.d = Double.NaN;
        p.s = text(); p.o = depth < 5 ? leaf(depth) : null; p.arr = rng.nextBoolean() ? new int[] { 1, -2, 3 } : null;
        if (p instanceof Q) { ((Q) p).extra = text(); ((Q) p).list = new ArrayList<>(List.of("a", rng.nextInt())); }
        return p;
    }

    static IdentityHashMap<Object, Integer> seen;

    static String prim(Field f, Object o) throws Exception {
        Class<?> t = f.getType();
        if (t == int.class) return Integer.toString(f.getInt(o));
        if (t == long.class) return Long.toString(f.getLong(o));
        if (t == short.class) return Short.toString(f.getShort(o));
        if (t == byte.class) return Byte.toString(f.getByte(o));
        if (t == char.class) return Integer.toString(f.getChar(o));
        if (t == boolean.class) return f.getBoolean(o) ? "1" : "0";
        if (t == float.class) return Integer.toUnsignedString(Float.floatToRawIntBits(f.getFloat(o)));
        return Long.toString(Double.doubleToRawLongBits(f.getDouble(o)));
    }

    static String primArray(Object a, int k) {
        Class<?> t = a.getClass().getComponentType();
        if (t == int.class) return Integer.toString(Array.getInt(a, k));
        if (t == long.class) return Long.toString(Array.getLong(a, k));
        if (t == short.class) return Short.toString(Array.getShort(a, k));
        if (t == byte.class) return Byte.toString(Array.getByte(a, k));
        if (t == char.class) return Integer.toString(Array.getChar(a, k));
        if (t == boolean.class) return Array.getBoolean(a, k) ? "1" : "0";
        if (t == float.class) return Integer.toUnsignedString(Float.floatToRawIntBits(Array.getFloat(a, k)));
        return Long.toString(Double.doubleToRawLongBits(Array.getDouble(a, k)));
    }

    static String dump(Object o) throws Exception {
        if (o == null) return "N";
        if (o instanceof String) return "S" + hs((String) o);
        if (o instanceof Enum) return "E" + ((Enum<?>) o).getDeclaringClass().getName() + "." + hs(((Enum<?>) o).name());
        if (o instanceof Class) return "C" + ((Class<?>) o).getName();
        Integer label = seen.get(o);
        if (label != null) return "^" + label;
        int me = seen.size();
        seen.put(o, me);
        Class<?> c = o.getClass();
        if (c.isArray()) {
            List<String> items = new ArrayList<>();
            for (int k = 0; k < Array.getLength(o); k++)
                items.add(c.getComponentType().isPrimitive() ? primArray(o, k) : dump(Array.get(o, k)));
            return "A" + me + c.getName() + "[" + String.join(",", items) + "]";
        }
        List<Class<?>> chain = new ArrayList<>();
        for (Class<?> k = c; k != null && Serializable.class.isAssignableFrom(k); k = k.getSuperclass()) chain.add(0, k);
        List<String> parts = new ArrayList<>();
        for (Class<?> k : chain) {
            ObjectStreamClass osc = ObjectStreamClass.lookup(k);
            if (osc == null || Externalizable.class.isAssignableFrom(k)) continue;
            for (ObjectStreamField sf : osc.getFields()) {
                Field f = k.getDeclaredField(sf.getName());
                f.setAccessible(true);
                parts.add(sf.getName() + "=" + (f.getType().isPrimitive() ? prim(f, o) : dump(f.get(o))));
            }
        }
        return "O" + me + c.getName() + "(" + String.join(",", parts) + ")";
    }

    static void generate(long seed, String dir, int n) throws Exception {
        rng = new Random(seed);
        for (int k = 0; k < n; k++) {
            ByteArrayOutputStream bo = new ByteArrayOutputStream();
            ObjectOutputStream out = new ObjectOutputStream(bo);
            int roots = 1 + rng.nextInt(3);
            List<Object> written = new ArrayList<>();
            for (int r = 0; r < roots; r++) {
                Object v = leaf(0);
                if (rng.nextInt(5) == 0) out.writeUnshared(v); else out.writeObject(v);
                written.add(v);
                if (rng.nextInt(4) == 0) { out.reset(); }
                if (rng.nextInt(4) == 0) { out.writeInt(rng.nextInt()); }
                if (rng.nextInt(6) == 0) { out.writeObject(String.class); written.add(String.class); }
            }
            out.close();
            Path p = Paths.get(dir, "j" + k + ".ser");
            Files.write(p, bo.toByteArray());
            // Read it back as Java does, and dump what was read.
            ObjectInputStream in = new ObjectInputStream(new ByteArrayInputStream(bo.toByteArray()));
            seen = new IdentityHashMap<>();
            List<String> dumps = new ArrayList<>();
            while (true) {
                try { dumps.add(dump(in.readObject())); }
                catch (OptionalDataException e) { if (e.eof) break; in.readFully(new byte[e.length]); }
                catch (EOFException e) { break; }
            }
            System.out.println(p + " " + String.join(";", dumps));
        }
    }

    public static void main(String[] a) throws Exception {
        if (a[0].equals("mf")) manifests(Arrays.copyOfRange(a, 1, a.length));
        else if (a[0].equals("jgen")) generate(Long.parseLong(a[1]), a[2], Integer.parseInt(a[3]));
    }
}
