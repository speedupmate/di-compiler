<?php
namespace Magento\Framework\Model;

class AbstractModel extends \Magento\Framework\DataObject
    implements \Magento\Framework\Model\ResourceModel\Db\ObjectRelationInterface,
               \Magento\Framework\DataObject\IdentityInterface
{
    public function __construct(
        \Magento\Framework\Model\Context $context,
        \Magento\Framework\Registry $registry,
        array $data = []
    ) {
        parent::__construct($data);
    }
}
