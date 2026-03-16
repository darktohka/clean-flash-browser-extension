package
{
    import flash.display.Sprite;
    import flash.text.TextField;

    public class Hello extends Sprite
    {
        public function Hello()
        {
            var tf:TextField = new TextField();
            tf.text = "Hello from mxmlc!";
            tf.width = 400;
            addChild(tf);
        }
    }
}